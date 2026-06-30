//! 发送端会话（PROTOCOL.md §7 左列）。

use std::sync::Arc;

use aa4c_proto::{client_hello, read_message, unexpected, write_message, Message};
use aa4c_types::{Aa4cError, DeviceId, Result, TaskId, TransferStatus};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

use crate::path::SendFile;
use crate::progress::Progress;
use crate::TransferService;

/// 单文件最大重传次数（PROTOCOL.md §7 规则 2）。
const MAX_RETRIES: u32 = 2;

pub(crate) struct SendJob {
    pub task_id: TaskId,
    pub peer_id: DeviceId,
    pub addr: std::net::SocketAddr,
    pub files: Vec<SendFile>,
    pub total: u64,
}

/// 发送任务入口：建连、驱动会话、收尾（状态与事件）。
pub(crate) async fn run(svc: Arc<TransferService>, job: SendJob, cancel: CancellationToken) {
    let result = drive(&svc, &job, &cancel).await;
    svc.finish_task(&job.task_id, result).await;
}

async fn drive(
    svc: &Arc<TransferService>,
    job: &SendJob,
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
        &Message::Offer {
            task_id: job.task_id.clone(),
            files: job.files.iter().map(|f| f.meta.clone()).collect(),
        },
    )
    .await?;

    // 等待接收方用户确认（可能较慢，单独使用同一超时）
    match timeout(t, read_message(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("offer answer timeout".into()))??
    {
        Message::OfferAnswer { accept: true, .. } => {}
        Message::OfferAnswer { accept: false, .. } => return Err(Aa4cError::TransferRejected),
        Message::Cancel { reason, .. } => {
            return Err(if reason == "not_paired" {
                Aa4cError::NotPaired(job.peer_id.clone())
            } else {
                Aa4cError::Network(format!("peer cancelled: {reason}"))
            });
        }
        other => return Err(unexpected(&other)),
    }

    svc.store
        .update_task_status(&job.task_id, TransferStatus::Transferring, None)
        .await?;

    let mut progress = Progress::new(
        job.task_id.clone(),
        svc.events.clone(),
        svc.store.clone(),
        job.total,
    );
    transfer_files(
        &mut stream,
        &job.task_id,
        &job.files,
        svc.config.chunk_size,
        t,
        &mut progress,
        cancel,
    )
    .await
}

/// 文件发送主循环（与连接建立解耦，便于协议级测试）。
async fn transfer_files<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    task_id: &TaskId,
    files: &[SendFile],
    chunk_size: usize,
    t: std::time::Duration,
    progress: &mut Progress,
    cancel: &CancellationToken,
) -> Result<()> {
    for (index, file) in files.iter().enumerate() {
        let index =
            u32::try_from(index).map_err(|_| Aa4cError::Protocol("too many files".into()))?;
        let mut attempts = 0u32;
        loop {
            if cancel.is_cancelled() {
                let _ = write_message(stream, &cancel_msg(task_id)).await;
                return Err(Aa4cError::Cancelled);
            }
            let sent = send_file(stream, index, file, chunk_size, progress, cancel).await?;
            write_message(
                stream,
                &Message::FileDone {
                    file_index: index,
                    hash: sent,
                },
            )
            .await?;
            match timeout(t, read_message(stream))
                .await
                .map_err(|_| Aa4cError::Network("file ack timeout".into()))??
            {
                Message::FileAck { ok: true, .. } => break,
                Message::FileAck { ok: false, .. } => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        // PROTOCOL.md §7 规则 2：重传仍失败 → 通知对端取消，任务 Failed
                        let _ = write_message(stream, &cancel_msg(task_id)).await;
                        return Err(Aa4cError::HashMismatch {
                            path: file.meta.rel_path.clone(),
                        });
                    }
                    tracing::warn!(file = %file.meta.rel_path, attempt = attempts, "hash mismatch, retrying");
                    progress.rollback(file.meta.size);
                }
                Message::Cancel { reason, .. } => {
                    return Err(Aa4cError::Network(format!("peer cancelled: {reason}")));
                }
                other => return Err(unexpected(&other)),
            }
        }
    }

    write_message(
        stream,
        &Message::TaskDone {
            task_id: task_id.clone(),
        },
    )
    .await?;
    progress.finalize().await;
    Ok(())
}

/// 流式发送单个文件：分块发送，边读边算 BLAKE3，返回整文件哈希。
async fn send_file<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    index: u32,
    file: &SendFile,
    chunk_size: usize,
    progress: &mut Progress,
    cancel: &CancellationToken,
) -> Result<String> {
    let mut reader = tokio::fs::File::open(&file.abs).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; chunk_size];
    let mut offset: u64 = 0;
    let mut remaining = file.meta.size;

    while remaining > 0 {
        if cancel.is_cancelled() {
            return Err(Aa4cError::Cancelled);
        }
        let want = usize::try_from(remaining.min(buf.len() as u64)).expect("chunk fits usize");
        reader.read_exact(&mut buf[..want]).await.map_err(|_| {
            Aa4cError::Protocol(format!(
                "file changed during transfer: {}",
                file.meta.rel_path
            ))
        })?;
        hasher.update(&buf[..want]);
        write_message(
            stream,
            &Message::Chunk {
                file_index: index,
                offset,
                len: u32::try_from(want).expect("chunk_size fits u32"),
            },
        )
        .await?;
        stream.write_all(&buf[..want]).await?;
        offset += want as u64;
        remaining -= want as u64;
        progress.add(want as u64, &file.meta.rel_path).await;
    }
    stream.flush().await?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn cancel_msg(task_id: &TaskId) -> Message {
    Message::Cancel {
        task_id: task_id.clone(),
        reason: "cancelled by sender".into(),
    }
}

/// 服务一次按需拉取（里程碑 4）：本端已读到对端 `FetchRequest` 并解析出共享文件，
/// 现在在**同一入站连接**上反转角色当发送方——记一条 Send 任务、回 `Offer`、收
/// `OfferAnswer` 后复用 [`transfer_files`] 把内容推给拉取方。收尾交由调用方 `finish_task`。
pub(crate) async fn serve_fetch<S: AsyncRead + AsyncWrite + Unpin>(
    svc: &Arc<TransferService>,
    stream: &mut S,
    peer_id: &DeviceId,
    task_id: &TaskId,
    resolved: crate::ResolvedFetch,
) -> Result<()> {
    use aa4c_proto::FileMeta;
    use aa4c_types::{Direction, FileStatus, TransferFile, TransferTask};

    let t = svc.config.timeout;
    let file = SendFile {
        abs: resolved.abs,
        meta: FileMeta {
            rel_path: resolved.rel_path.clone(),
            size: resolved.size,
        },
    };
    let total = resolved.size;

    // B 侧记一条 Send 任务：拉取在 B 的「记录」里呈现为「发送」，progress/收尾沿用既有机制
    svc.store
        .insert_task(&TransferTask {
            id: task_id.clone(),
            direction: Direction::Send,
            peer: peer_id.clone(),
            files: vec![TransferFile {
                rel_path: file.meta.rel_path.clone(),
                size: file.meta.size,
                hash: None,
                status: FileStatus::Pending,
            }],
            status: TransferStatus::WaitingAccept,
            total_bytes: total,
            transferred_bytes: 0,
            created_at: crate::now_ms(),
            error: None,
        })
        .await?;
    let cancel = svc.register_cancel(task_id);

    write_message(
        stream,
        &Message::Offer {
            task_id: task_id.clone(),
            files: vec![file.meta.clone()],
        },
    )
    .await?;
    match timeout(t, read_message(stream))
        .await
        .map_err(|_| Aa4cError::Network("offer answer timeout".into()))??
    {
        Message::OfferAnswer { accept: true, .. } => {}
        Message::OfferAnswer { accept: false, .. } => return Err(Aa4cError::TransferRejected),
        Message::Cancel { reason, .. } => {
            return Err(Aa4cError::Network(format!("peer cancelled: {reason}")));
        }
        other => return Err(unexpected(&other)),
    }

    svc.store
        .update_task_status(task_id, TransferStatus::Transferring, None)
        .await?;
    let mut progress = Progress::new(
        task_id.clone(),
        svc.events.clone(),
        svc.store.clone(),
        total,
    );
    transfer_files(
        stream,
        task_id,
        &[file],
        svc.config.chunk_size,
        t,
        &mut progress,
        &cancel,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_proto::FileMeta;
    use std::time::Duration;

    /// 脚本化假接收端：前 `fail_acks` 次 FileDone 回 Ack(false)，其余回 true。
    /// 返回（收到的 FileDone 次数, 是否收到 TaskDone）。
    async fn fake_receiver<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
        mut fail_acks: u32,
    ) -> (u32, bool) {
        let mut done_count = 0u32;
        loop {
            match read_message(stream).await {
                Ok(Message::Chunk { len, .. }) => {
                    let mut buf = vec![0u8; len as usize];
                    stream.read_exact(&mut buf).await.unwrap();
                }
                Ok(Message::FileDone { file_index, .. }) => {
                    done_count += 1;
                    let ok = if fail_acks > 0 {
                        fail_acks -= 1;
                        false
                    } else {
                        true
                    };
                    write_message(stream, &Message::FileAck { file_index, ok })
                        .await
                        .unwrap();
                }
                Ok(Message::TaskDone { .. }) => return (done_count, true),
                Ok(Message::Cancel { .. }) | Err(_) => return (done_count, false),
                Ok(other) => panic!("unexpected message: {other:?}"),
            }
        }
    }

    /// 在内存 duplex 上驱动发送循环，返回（发送结果, FileDone 次数, TaskDone）。
    async fn run_sender(file_size: usize, fail_acks: u32) -> (Result<()>, u32, bool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        std::fs::write(&path, vec![7u8; file_size]).unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("db.sqlite"))
            .await
            .unwrap();
        let (events, _keep) = tokio::sync::broadcast::channel(16);
        let mut progress = Progress::new("t1".into(), events, store, file_size as u64);
        let files = vec![SendFile {
            abs: path,
            meta: FileMeta {
                rel_path: "data.bin".into(),
                size: file_size as u64,
            },
        }];
        let (mut a, mut b) = tokio::io::duplex(256 * 1024);
        let cancel = CancellationToken::new();
        let task_id = "t1".to_string();
        let (send_res, (done_count, task_done)) = tokio::join!(
            transfer_files(
                &mut a,
                &task_id,
                &files,
                1024, // 小分块，确保多块
                Duration::from_secs(5),
                &mut progress,
                &cancel,
            ),
            fake_receiver(&mut b, fail_acks),
        );
        (send_res, done_count, task_done)
    }

    #[tokio::test]
    async fn retransmits_after_single_hash_mismatch() {
        let (result, done_count, task_done) = run_sender(3000, 1).await;
        result.unwrap();
        assert_eq!(done_count, 2, "首次失败 + 重传成功");
        assert!(task_done);
    }

    #[tokio::test]
    async fn gives_up_after_two_retries() {
        let (result, done_count, task_done) = run_sender(3000, 3).await;
        assert!(
            matches!(result, Err(Aa4cError::HashMismatch { .. })),
            "{result:?}"
        );
        assert_eq!(done_count, 3, "1 次原始 + 2 次重传后放弃");
        assert!(!task_done);
    }
}
