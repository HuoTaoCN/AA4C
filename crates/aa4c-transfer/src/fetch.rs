//! 按需拉取的发起端（SYNC_DESIGN.md §4，里程碑 4；里程碑 C6 起也承载分享链接打开）。
//!
//! 拉取方 A 主动连对端 B，握手后发 `FetchRequest{rel_path}`（完全信任设备的按需拉取）
//! 或 `ShareRequest{token}`（打开分享链接，见 [`FetchTarget`]）；B 校验通过后反转角色回
//! `Offer`，A 自动接受（是自己要的）并复用既有接收循环 [`crate::recv::receive_files`]
//! 落盘。落地到 `save_dir`（缺省 Inbox），随后本机扫描把它标绿（与远端同限定路径时即
//! 并入同一条目；分享拉取的落点由 Core 决定，通常也是 Inbox）。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_proto::{client_hello, read_message, unexpected, write_message, Message};
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, Direction, FileStatus, Result, TaskId, TransferFile,
    TransferStatus, TransferTask,
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::path::sanitize_rel_path;
use crate::{now_ms, TransferService};

/// 拉取任务的请求方式：按限定路径（完全信任设备的按需拉取）或按分享 token
/// （打开分享链接，里程碑 C6）。两者在对端反转角色回 `Offer` 之后走完全相同的收尾流程。
pub(crate) enum FetchTarget {
    Path(String),
    Share(String),
}

pub(crate) struct FetchJob {
    pub task_id: TaskId,
    pub peer_id: DeviceId,
    /// `None`：连接阶梯前三档都没解析出地址，直接尝试中继（里程碑 C4，见 `TransferService::dial`）。
    pub addr: Option<std::net::SocketAddr>,
    pub target: FetchTarget,
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
    let (mut stream, via) = svc.dial(&job.peer_id, job.addr).await?;
    let _ = svc.events.send(CoreEvent::TransferConnected {
        task_id: job.task_id.clone(),
        via,
    });

    let (hello_id, proto) = client_hello(&mut stream, svc.identity.device_id()).await?;
    if hello_id != job.peer_id {
        return Err(Aa4cError::Protocol("hello id != expected peer".into()));
    }
    let request = match &job.target {
        FetchTarget::Path(rel_path) => {
            // 版本门槛：对端为 v1（proto<2）不支持按需拉取，直接不发 FetchRequest（优雅降级）
            if proto < aa4c_types::SYNC_PROTO_VERSION {
                return Err(Aa4cError::Protocol("对端版本过旧，无法拉取".into()));
            }
            Message::FetchRequest {
                rel_path: rel_path.clone(),
            }
        }
        FetchTarget::Share(token) => {
            // 版本门槛同理：对端不认识 ShareRequest 就不发（里程碑 C6）
            if proto < aa4c_types::SHARE_PROTO_VERSION {
                return Err(Aa4cError::Protocol("对端版本过旧，不支持分享链接".into()));
            }
            Message::ShareRequest {
                token: token.clone(),
            }
        }
    };
    write_message(&mut stream, &request).await?;

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

    // A 侧 Recv 记录（拉取在「记录」里呈现为接收）。`transfer_tasks.peer_device_id` 有外键
    // 约束（REFERENCES devices），只对**已知设备**成立；打开分享链接（里程碑 C6）允许对方
    // 是从未配对过的设备，这种情况下插入会违反外键——跳过任务落库（同 `send::serve_fetch`
    // 的处理，这次传输不会出现在本机「记录」页，但协议本身不受影响）。
    if svc.store.get_device(&job.peer_id).await?.is_some() {
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
    }

    // 自动接受（是自己发起的拉取）——回声 B 的 wire task_id
    write_message(
        &mut stream,
        &Message::OfferAnswer {
            task_id: wire_task_id,
            accept: true,
        },
    )
    .await?;

    // 拉取路径暂不支持续传（C1 范围内，见 V0.3_IMPLEMENTATION_PLAN.md C1 备注），恒传空切片。
    crate::recv::receive_files(
        svc,
        &mut stream,
        &job.task_id,
        &[meta],
        &[rel],
        &save_dir,
        &[],
        cancel,
    )
    .await
}
