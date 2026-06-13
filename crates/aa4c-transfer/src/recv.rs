//! 接收端会话（PROTOCOL.md §7 右列）。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_identity::device_id_from_cert;
use aa4c_proto::{read_message, server_hello, unexpected, write_message, FileMeta, Message};
use aa4c_types::{
    Aa4cError, CoreEvent, Direction, FileStatus, Result, TaskId, TransferFile, TransferStatus,
    TransferTask,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::path::{dedup_target, sanitize_rel_path};
use crate::progress::Progress;
use crate::TransferService;

type TlsServerStream = tokio_rustls::server::TlsStream<TcpStream>;

/// 入站连接入口：握手、trusted 校验、按首消息分流。
pub(crate) async fn run_incoming(
    svc: Arc<TransferService>,
    mut stream: TlsServerStream,
) -> Result<()> {
    let t = svc.config.timeout;
    let cert_id = {
        let certs = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| Aa4cError::Protocol("peer presented no certificate".into()))?;
        device_id_from_cert(certs)?
    };
    let (hello_id, _proto) = server_hello(&mut stream, svc.identity.device_id()).await?;
    if hello_id != cert_id {
        return Err(Aa4cError::Protocol("hello id != certificate id".into()));
    }
    let trusted = svc
        .store
        .get_device(&cert_id)
        .await?
        .map(|d| d.trusted)
        .unwrap_or(false);

    let first = timeout(t, read_message(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("first message timeout".into()))??;
    match first {
        Message::Offer { task_id, files } => {
            if !trusted {
                // PROTOCOL §5 规则 3：未配对设备直接拒绝
                let _ = write_message(
                    &mut stream,
                    &Message::Cancel {
                        task_id,
                        reason: "not_paired".into(),
                    },
                )
                .await;
                return Err(Aa4cError::NotPaired(cert_id));
            }
            let result = session(&svc, &mut stream, &cert_id, task_id.clone(), files).await;
            svc.finish_task(&task_id, result).await;
            Ok(())
        }
        // 配对连接的分流在 M6 接线（PairingManager::handle_incoming）
        Message::PairRequest { .. } => {
            let _ = write_message(
                &mut stream,
                &Message::PairReject {
                    reason: "pairing dispatch not wired".into(),
                },
            )
            .await;
            Err(Aa4cError::Protocol(
                "pairing not dispatched here yet".into(),
            ))
        }
        other => Err(unexpected(&other)),
    }
}

/// 单个文件的落盘状态。
struct OpenFile {
    index: u32,
    part: PathBuf,
    target: PathBuf,
    file: tokio::fs::File,
    hasher: blake3::Hasher,
    written: u64,
}

/// 接收会话主循环。
async fn session(
    svc: &Arc<TransferService>,
    stream: &mut TlsServerStream,
    peer_id: &str,
    task_id: TaskId,
    files: Vec<FileMeta>,
) -> Result<()> {
    let t = svc.config.timeout;

    // 路径净化（任何非法路径 → 整个任务拒绝）
    let mut rel_paths = Vec::with_capacity(files.len());
    for f in &files {
        rel_paths.push(sanitize_rel_path(&f.rel_path)?);
    }
    let total: u64 = files.iter().map(|f| f.size).sum();

    // 入库 + 通知 UI
    let task = TransferTask {
        id: task_id.clone(),
        direction: Direction::Recv,
        peer: peer_id.to_string(),
        files: files
            .iter()
            .map(|f| TransferFile {
                rel_path: f.rel_path.clone(),
                size: f.size,
                hash: None,
                status: FileStatus::Pending,
            })
            .collect(),
        status: TransferStatus::WaitingAccept,
        total_bytes: total,
        transferred_bytes: 0,
        created_at: crate::now_ms(),
        error: None,
    };
    svc.store.insert_task(&task).await?;
    let cancel = svc.register_cancel(&task_id);
    let decision_rx = svc.register_pending_accept(&task_id);
    let _ = svc.events.send(CoreEvent::TransferRequest { task });

    // 等待用户决定（独立超时；svc.accept() 注入）
    let (accepted, save_dir) = match timeout(t, decision_rx).await {
        Ok(Ok(decision)) => decision,
        Ok(Err(_)) => return Err(Aa4cError::Cancelled),
        Err(_) => {
            let _ = write_message(stream, &answer(&task_id, false)).await;
            return Err(Aa4cError::Network("accept timeout".into()));
        }
    };
    if !accepted {
        write_message(stream, &answer(&task_id, false)).await?;
        return Err(Aa4cError::TransferRejected);
    }
    let save_dir = save_dir.unwrap_or_else(|| svc.config.default_save_dir.clone());
    tokio::fs::create_dir_all(&save_dir).await?;
    write_message(stream, &answer(&task_id, true)).await?;
    svc.store
        .update_task_status(&task_id, TransferStatus::Transferring, None)
        .await?;

    let mut progress = Progress::new(
        task_id.clone(),
        svc.events.clone(),
        svc.store.clone(),
        total,
    );
    let mut done = vec![false; files.len()];
    let mut current: Option<OpenFile> = None;
    let mut parts_created: Vec<PathBuf> = Vec::new();
    let max_chunk = svc.config.chunk_size * 2;

    let result = loop {
        let msg = tokio::select! {
            () = cancel.cancelled() => {
                let _ = write_message(stream, &Message::Cancel {
                    task_id: task_id.clone(),
                    reason: "cancelled by receiver".into(),
                }).await;
                break Err(Aa4cError::Cancelled);
            }
            m = timeout(t, read_message(stream)) => match m {
                Ok(inner) => match inner { Ok(msg) => msg, Err(e) => break Err(e) },
                Err(_) => break Err(Aa4cError::Network("peer silent too long".into())),
            },
        };
        match msg {
            Message::Chunk {
                file_index,
                offset,
                len,
            } => {
                let idx = file_index as usize;
                if idx >= files.len() || done[idx] || len as usize > max_chunk {
                    break Err(Aa4cError::Protocol("invalid chunk header".into()));
                }
                // 新文件或重传重启（offset 0 且无打开文件）
                if current.as_ref().map(|c| c.index) != Some(file_index) {
                    if current.is_some() || offset != 0 {
                        break Err(Aa4cError::Protocol("out-of-order chunk".into()));
                    }
                    let open = open_part(&save_dir, &rel_paths[idx], file_index).await;
                    match open {
                        Ok(open) => {
                            parts_created.push(open.part.clone());
                            current = Some(open);
                        }
                        Err(e) => break Err(e),
                    }
                }
                let open = current.as_mut().expect("just ensured");
                if offset != open.written {
                    break Err(Aa4cError::Protocol("chunk offset mismatch".into()));
                }
                let mut buf = vec![0u8; len as usize];
                if let Err(e) = stream.read_exact(&mut buf).await {
                    break Err(e.into());
                }
                open.hasher.update(&buf);
                if let Err(e) = open.file.write_all(&buf).await {
                    break Err(e.into());
                }
                open.written += u64::from(len);
                progress.add(u64::from(len), &files[idx].rel_path).await;
            }
            Message::FileDone { file_index, hash } => {
                let idx = file_index as usize;
                if idx >= files.len() || done[idx] {
                    break Err(Aa4cError::Protocol("unexpected FileDone".into()));
                }
                // 空文件：没有任何 Chunk，直接建空文件
                if current.as_ref().map(|c| c.index) != Some(file_index) {
                    if current.is_some() || files[idx].size != 0 {
                        break Err(Aa4cError::Protocol("FileDone without chunks".into()));
                    }
                    match open_part(&save_dir, &rel_paths[idx], file_index).await {
                        Ok(open) => {
                            parts_created.push(open.part.clone());
                            current = Some(open);
                        }
                        Err(e) => break Err(e),
                    }
                }
                let open = current.take().expect("ensured above");
                let written = open.written;
                let ok = written == files[idx].size
                    && open.hasher.finalize().to_hex().to_string() == hash;
                if ok {
                    if let Err(e) = finalize_file(open).await {
                        break Err(e);
                    }
                    done[idx] = true;
                } else {
                    // 哈希/长度不符：丢弃本次写入，等待发送方重传（part 下次截断重写）
                    tracing::warn!(file = %files[idx].rel_path, "hash mismatch, requesting resend");
                    drop(open);
                }
                if let Err(e) = write_message(stream, &Message::FileAck { file_index, ok }).await {
                    break Err(e);
                }
                if !ok {
                    progress.rollback(written);
                }
            }
            Message::TaskDone { .. } => {
                if done.iter().all(|d| *d) {
                    progress.finalize().await;
                    break Ok(());
                }
                break Err(Aa4cError::Protocol("TaskDone before all files".into()));
            }
            Message::Cancel { reason, .. } => {
                break Err(Aa4cError::Network(format!("peer cancelled: {reason}")));
            }
            other => break Err(unexpected(&other)),
        }
    };

    if result.is_err() {
        cleanup_parts(&parts_created).await;
    }
    result
}

fn answer(task_id: &TaskId, accept: bool) -> Message {
    Message::OfferAnswer {
        task_id: task_id.clone(),
        accept,
    }
}

/// 打开（或截断重建）part 文件。
async fn open_part(
    save_dir: &std::path::Path,
    rel: &std::path::Path,
    index: u32,
) -> Result<OpenFile> {
    let target = save_dir.join(rel);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = target.with_file_name(format!("{file_name}.aa4c-part"));
    let file = tokio::fs::File::create(&part).await?; // create == truncate，重传自然重置
    Ok(OpenFile {
        index,
        part,
        target,
        file,
        hasher: blake3::Hasher::new(),
        written: 0,
    })
}

/// 校验通过：落正式文件名（重名自动加 ` (1)`）。
async fn finalize_file(mut open: OpenFile) -> Result<()> {
    open.file.flush().await?;
    drop(open.file);
    let target = dedup_target(&open.target);
    tokio::fs::rename(&open.part, &target).await?;
    Ok(())
}

/// 会话失败：清理遗留 part 文件（PROTOCOL.md §7 规则 3）。
async fn cleanup_parts(parts: &[PathBuf]) {
    for part in parts {
        let _ = tokio::fs::remove_file(part).await;
    }
}
