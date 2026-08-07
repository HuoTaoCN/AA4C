//! 发送端会话（PROTOCOL.md §7 左列）。

use std::collections::HashMap;
use std::sync::Arc;

use aa4c_proto::{client_hello, read_message, unexpected, write_message, Message};
use aa4c_types::{Aa4cError, CoreEvent, DeviceId, Result, TaskId, TransferStatus};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

use crate::path::SendFile;
use crate::progress::Progress;
use crate::{StopSignal, TransferService};

/// 单文件最大重传次数（PROTOCOL.md §7 规则 2）。
const MAX_RETRIES: u32 = 2;

pub(crate) struct SendJob {
    pub task_id: TaskId,
    pub peer_id: DeviceId,
    /// `None`：连接阶梯前三档都没解析出地址，直接尝试中继（里程碑 C3，见 `TransferService::dial`）。
    pub addr: Option<std::net::SocketAddr>,
    pub files: Vec<SendFile>,
    pub total: u64,
}

/// 发送任务入口：建连、驱动会话、收尾（状态与事件）。
pub(crate) async fn run(svc: Arc<TransferService>, job: SendJob, signal: StopSignal) {
    let result = drive(&svc, &job, &signal).await;
    // 暂停走的也是 `Cancelled` 这条控制流（发送循环靠它退出），但收尾完全不同：
    // 不是终态、要留着发送清单供「继续」用，见 `finish_paused_task`。
    if matches!(result, Err(Aa4cError::Cancelled)) && signal.is_paused() {
        svc.finish_paused_task(&job.task_id).await;
    } else {
        svc.finish_task(&job.task_id, result).await;
    }
}

async fn drive(svc: &Arc<TransferService>, job: &SendJob, signal: &StopSignal) -> Result<()> {
    let t = svc.config.timeout;
    let (mut stream, via) = svc.dial(&job.peer_id, job.addr).await?;
    // 里程碑 C4 连接质量：只在这里报一次，UI 据此显示「直连/中继（较慢）」（不落库，见
    // `CoreEvent::TransferConnected` 文档）。
    let _ = svc.events.send(CoreEvent::TransferConnected {
        task_id: job.task_id.clone(),
        via,
    });

    let (hello_id, proto) = client_hello(&mut stream, svc.identity.device_id()).await?;
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

    // 断点续传（PROTOCOL.md §13，里程碑 C1）：proto ≥ RESUME_PROTO_VERSION 时接收方
    // **确定性**紧跟一条 ResumeReport（不是尝试性的），两端都是同一批代码，不会缺席。
    let resume: HashMap<u32, u64> = if proto >= aa4c_types::RESUME_PROTO_VERSION {
        match timeout(t, read_message(&mut stream))
            .await
            .map_err(|_| Aa4cError::Network("resume report timeout".into()))??
        {
            Message::ResumeReport { progress, .. } => progress
                .into_iter()
                .map(|p| (p.file_index, p.verified_bytes))
                .collect(),
            other => return Err(unexpected(&other)),
        }
    } else {
        HashMap::new()
    };

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
        signal,
        &resume,
    )
    .await
}

/// 文件发送主循环（与连接建立解耦，便于协议级测试）。
///
/// `resume` 为每个文件的续传起点（file_index → verified_bytes，里程碑 C1）；
/// 空表示不续传（V0.2 及更早对端，或按需拉取路径，行为与之前完全一致）。
#[allow(clippy::too_many_arguments)]
async fn transfer_files<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    task_id: &TaskId,
    files: &[SendFile],
    chunk_size: usize,
    t: std::time::Duration,
    progress: &mut Progress,
    signal: &StopSignal,
    resume: &HashMap<u32, u64>,
) -> Result<()> {
    for (index, file) in files.iter().enumerate() {
        let index =
            u32::try_from(index).map_err(|_| Aa4cError::Protocol("too many files".into()))?;
        let resume_from = resume.get(&index).copied().unwrap_or(0);
        if resume_from > 0 {
            // 续传前缀记一次进度；此文件后续的重试不再重复计（send_file 本身不碰 progress）
            progress.add(resume_from, &file.meta.rel_path).await;
        }
        let mut attempts = 0u32;
        loop {
            if signal.is_stopped() {
                // 暂停时**不发** `Cancel`——静默断开才会让接收端保留 .aa4c-part
                // 供续传（PROTOCOL.md §7 规则 3，见 `StopSignal` 文档）。
                if !signal.is_paused() {
                    let _ = write_message(stream, &cancel_msg(task_id)).await;
                }
                return Err(Aa4cError::Cancelled);
            }
            let sent = match send_file(
                stream,
                index,
                file,
                chunk_size,
                progress,
                signal,
                resume_from,
            )
            .await
            {
                Ok(sent) => sent,
                // send_file 内部（分块途中）检测到取消：这里补发 Cancel 通知对端，
                // 否则对端只会看到连接静默直至超时——PROTOCOL.md §7 规则 3 要求
                // 明确取消要通知对端清理 part 文件（V0.3 起未通知的中断会保留
                // part 文件供续传，见 recv.rs 的 explicit_cancel）。
                Err(Aa4cError::Cancelled) => {
                    if !signal.is_paused() {
                        let _ = write_message(stream, &cancel_msg(task_id)).await;
                    }
                    return Err(Aa4cError::Cancelled);
                }
                Err(e) => return Err(e),
            };
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
                    // 只回退本次真正发送的部分：续传前缀的 credit 已经一次性记入且不重复，
                    // 不应该被这里的重试回退掉。
                    progress.rollback(file.meta.size - resume_from);
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
///
/// `resume_from > 0`：先重新流式读取源文件同样长度的前缀喂 hasher（保持整文件哈希
/// 正确），从该偏移开始才真正通过网络发送——不在这里触碰 `progress`（调用方在
/// [`transfer_files`] 里对每个文件只记一次续传前缀的进度，重试不重复计）。
async fn send_file<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    index: u32,
    file: &SendFile,
    chunk_size: usize,
    progress: &mut Progress,
    signal: &StopSignal,
    resume_from: u64,
) -> Result<String> {
    let mut reader = tokio::fs::File::open(&file.abs).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; chunk_size];
    let mut offset: u64 = 0;

    if resume_from > 0 {
        let mut remaining = resume_from;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            reader.read_exact(&mut buf[..want]).await.map_err(|_| {
                Aa4cError::Protocol(format!(
                    "file changed during transfer: {}",
                    file.meta.rel_path
                ))
            })?;
            hasher.update(&buf[..want]);
            remaining -= want as u64;
        }
        offset = resume_from;
    }

    let mut remaining = file.meta.size - offset;
    while remaining > 0 {
        if signal.is_stopped() {
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
///
/// 拉取路径暂不支持续传（C1 范围内，见 V0.3_IMPLEMENTATION_PLAN.md C1 备注），恒传空表。
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

    // B 侧记一条 Send 任务：拉取在 B 的「记录」里呈现为「发送」，progress/收尾沿用既有机制。
    // `transfer_tasks.peer_device_id` 有外键约束（REFERENCES devices），只对**已知设备**
    // （已配对，无论是否完全信任）成立；分享链接（里程碑 C6）允许从未配对过的设备访问，
    // 这种情况下插入会违反外键——跳过任务落库（转而只留 `share_access` 审计记录，见
    // `aa4c-core::dispatch::ShareServe`），协议本身（Offer/Chunk/FileDone）不受影响，
    // 只是这次传输不会出现在 B 的「记录」页。
    let peer_known = svc.store.get_device(peer_id).await?.is_some();
    if peer_known {
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
    }
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

    if peer_known {
        svc.store
            .update_task_status(task_id, TransferStatus::Transferring, None)
            .await?;
    }
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
        &HashMap::new(),
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
        let signal = StopSignal::new_for_test();
        let task_id = "t1".to_string();
        let resume = HashMap::new();
        let (send_res, (done_count, task_done)) = tokio::join!(
            transfer_files(
                &mut a,
                &task_id,
                &files,
                1024, // 小分块，确保多块
                Duration::from_secs(5),
                &mut progress,
                &signal,
                &resume,
            ),
            fake_receiver(&mut b, fail_acks),
        );
        (send_res, done_count, task_done)
    }

    /// 在**传输途中**叫停，返回（发送结果, 接收端有没有收到 `Cancel`）。
    ///
    /// "途中"是确定性的、不靠 sleep 赌时序：假接收端读满 3 个分块之后才触发叫停，
    /// 此时发送循环必然还在文件中间。之后继续读，看有没有 `Cancel` 跟过来。
    async fn stop_midway_and_report_cancel(pause: bool) -> (Result<()>, bool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        std::fs::write(&path, vec![9u8; 512 * 1024]).unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("db.sqlite"))
            .await
            .unwrap();
        let (events, _keep) = tokio::sync::broadcast::channel(16);
        let mut progress = Progress::new("t1".into(), events, store, 512 * 1024);
        let files = vec![SendFile {
            abs: path,
            meta: FileMeta {
                rel_path: "data.bin".into(),
                size: 512 * 1024,
            },
        }];
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let signal = StopSignal::new_for_test();
        let trigger = signal.clone();
        let task_id = "t1".to_string();
        let resume = HashMap::new();

        let reader = async {
            let mut chunks = 0u32;
            loop {
                match read_message(&mut b).await {
                    Ok(Message::Chunk { len, .. }) => {
                        let mut buf = vec![0u8; len as usize];
                        if b.read_exact(&mut buf).await.is_err() {
                            return false;
                        }
                        chunks += 1;
                        if chunks == 3 {
                            if pause {
                                trigger.pause();
                            } else {
                                trigger.cancel();
                            }
                        }
                    }
                    Ok(Message::Cancel { .. }) => return true,
                    // 对端把连接丢了——静默断开，正是暂停期望的形态。
                    Err(_) => return false,
                    Ok(_) => {}
                }
            }
        };
        // 暂停路径下发送方直接返回、不再写任何东西，而 `a` 还活着（同一作用域里
        // 借着），读端等不到 EOF 会一直挂着——所以给读端一个上限：超时即"没等到
        // Cancel"，正是我们要断言的结论。
        let (send_res, saw_cancel) = tokio::join!(
            transfer_files(
                &mut a,
                &task_id,
                &files,
                1024,
                Duration::from_secs(5),
                &mut progress,
                &signal,
                &resume,
            ),
            async {
                tokio::time::timeout(Duration::from_secs(2), reader)
                    .await
                    .unwrap_or(false)
            },
        );
        (send_res, saw_cancel)
    }

    /// **暂停不能发 `Cancel`**——整套「暂停/继续」就靠这一点成立：接收端只有在
    /// 没收到 `Cancel` 时才保留 `.aa4c-part`（PROTOCOL.md §7 规则 3），发了就等于
    /// 让对端把续传的本钱删掉。
    #[tokio::test]
    async fn pausing_does_not_send_cancel_to_the_peer() {
        let (result, saw_cancel) = stop_midway_and_report_cancel(true).await;
        assert!(matches!(result, Err(Aa4cError::Cancelled)));
        assert!(
            !saw_cancel,
            "pause must disconnect silently; sending Cancel would make the peer delete the .aa4c-part"
        );
    }

    /// 对照组：取消**必须**发 `Cancel`，对端据此清理半成品。
    #[tokio::test]
    async fn cancelling_does_send_cancel_to_the_peer() {
        let (result, saw_cancel) = stop_midway_and_report_cancel(false).await;
        assert!(matches!(result, Err(Aa4cError::Cancelled)));
        assert!(saw_cancel, "cancel must tell the peer so it cleans up");
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
