//! 按需拉取的发起端（SYNC_DESIGN.md §4，里程碑 4）。
//!
//! 拉取方 A 主动连对端 B，握手后发 `FetchRequest{rel_path}`；B 校验完全信任 + 路径
//! 在共享范围内后反转角色回 `Offer`，A 自动接受（是自己要的）并复用既有接收循环
//! [`crate::recv::receive_files`] 落盘。落地到 `save_dir`（缺省 Inbox），随后本机扫描
//! 把它标绿（与远端同限定路径时即并入同一条目）。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_proto::{client_hello, read_message, unexpected, write_message, Message};
use aa4c_types::{
    Aa4cError, DeviceId, Direction, FileStatus, Result, TaskId, TransferFile, TransferStatus,
    TransferTask,
};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

use crate::path::sanitize_rel_path;
use crate::{now_ms, TransferService};

pub(crate) struct FetchJob {
    pub task_id: TaskId,
    pub peer_id: DeviceId,
    pub addr: std::net::SocketAddr,
    /// 统一视图里的限定展示路径（顶层段为来源分组）。
    pub rel_path: String,
    pub save_dir: Option<PathBuf>,
}

/// 拉取任务入口：建连、驱动、收尾（finish_task 落状态 + 事件；成功事件 `TransferDone`
/// 会触发本机重扫，使新文件转绿）。
pub(crate) async fn run(svc: Arc<TransferService>, job: FetchJob) {
    let cancel = svc.register_cancel(&job.task_id);
    let result = drive(&svc, &job, &cancel).await;
    svc.finish_task(&job.task_id, result).await;
}

async fn drive(
    svc: &Arc<TransferService>,
    job: &FetchJob,
    cancel: &CancellationToken,
) -> Result<()> {
    let t = svc.config.timeout;
    let tcp = timeout(t, TcpStream::connect(job.addr))
        .await
        .map_err(|_| Aa4cError::Network("connect timeout".into()))??;
    let config = svc.identity.tls_client_config(Some(&job.peer_id))?;
    let mut stream = TlsConnector::from(Arc::new(config))
        .connect(
            tokio_rustls::rustls::pki_types::ServerName::try_from("aa4c").expect("static name"),
            tcp,
        )
        .await?;

    let (hello_id, _proto) = client_hello(&mut stream, svc.identity.device_id()).await?;
    if hello_id != job.peer_id {
        return Err(Aa4cError::Protocol("hello id != expected peer".into()));
    }
    write_message(
        &mut stream,
        &Message::FetchRequest {
            rel_path: job.rel_path.clone(),
        },
    )
    .await?;

    // B 反转角色回 Offer（或 Cancel 拒绝）
    let (wire_task_id, files) = match timeout(t, read_message(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("offer timeout".into()))??
    {
        Message::Offer { task_id, files } => (task_id, files),
        Message::Cancel { reason, .. } => {
            return Err(Aa4cError::Network(format!("peer refused fetch: {reason}")));
        }
        other => return Err(unexpected(&other)),
    };
    if files.len() != 1 {
        return Err(Aa4cError::Protocol("fetch expects exactly one file".into()));
    }
    let meta = files[0].clone();
    let total = meta.size;

    // 落盘相对路径：剥掉顶层来源分组段，落进 Inbox（缺省）。剥不掉则原样（保守）。
    let stripped = meta
        .rel_path
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(meta.rel_path.as_str());
    let rel = sanitize_rel_path(stripped)?;
    let save_dir = job
        .save_dir
        .clone()
        .unwrap_or_else(|| svc.config.default_save_dir.clone());
    tokio::fs::create_dir_all(&save_dir).await?;

    // A 侧 Recv 记录（拉取在「记录」里呈现为接收）
    svc.store
        .insert_task(&TransferTask {
            id: job.task_id.clone(),
            direction: Direction::Recv,
            peer: job.peer_id.clone(),
            files: vec![TransferFile {
                rel_path: meta.rel_path.clone(),
                size: meta.size,
                hash: None,
                status: FileStatus::Pending,
            }],
            status: TransferStatus::Transferring,
            total_bytes: total,
            transferred_bytes: 0,
            created_at: now_ms(),
            error: None,
        })
        .await?;

    // 自动接受（是自己发起的拉取）——回声 B 的 wire task_id
    write_message(
        &mut stream,
        &Message::OfferAnswer {
            task_id: wire_task_id,
            accept: true,
        },
    )
    .await?;

    crate::recv::receive_files(
        svc,
        &mut stream,
        &job.task_id,
        &[meta],
        &[rel],
        &save_dir,
        cancel,
    )
    .await
}
