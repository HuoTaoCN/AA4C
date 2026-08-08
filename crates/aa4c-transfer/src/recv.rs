//! 接收端会话（PROTOCOL.md §7 右列）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aa4c_identity::device_id_from_cert;
use aa4c_proto::{
    read_message, server_hello, unexpected, write_message, FileMeta, FileProgress, Message,
};
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, Direction, FileStatus, Result, TaskId, TransferFile,
    TransferStatus, TransferTask,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use crate::path::{dedup_target, sanitize_rel_path};
use crate::progress::Progress;
use crate::{SharedStream, TransferService};

type TlsServerStream = tokio_rustls::server::TlsStream<TcpStream>;

/// TCP+TLS 入站连接入口：握手、trusted 校验、按首消息分流。
///
/// `PairRequest` 走这里专属处理（配对目前只支持局域网 TCP，未纳入 QUIC，见
/// CONNECT_DESIGN.md）；其余消息（Offer/IndexRequest/FetchRequest）委托给
/// [`dispatch_shared`]，与 QUIC 入站（[`run_incoming_quic`]）共用同一套逻辑。
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
    let (hello_id, proto) = server_hello(&mut stream, svc.identity.device_id()).await?;
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

    // 配对：仅此路径支持（走具体的 TCP+TLS 类型，PairingManager 需要读取 TCP 对端地址）。
    if let Message::PairRequest { device, public_key } = first {
        return match svc.pair_dispatch.get() {
            Some(dispatch) => {
                dispatch.dispatch(stream, cert_id, device, public_key, proto);
                Ok(())
            }
            None => {
                let _ = write_message(
                    &mut stream,
                    &Message::PairReject {
                        reason: "pairing not available".into(),
                    },
                )
                .await;
                Err(Aa4cError::Protocol("pairing dispatch not wired".into()))
            }
        };
    }

    dispatch_shared(svc, Box::new(stream), cert_id, trusted, proto, first).await
}

/// QUIC 入站连接入口：接受第一条 bidi 流、握手、trusted 校验，走与 TCP 相同的共享分发
/// （里程碑 C1，CONNECT_DESIGN.md §5）。配对不支持 QUIC，遇到 `PairRequest` 直接拒绝
/// （见 [`dispatch_shared`]）。
pub(crate) async fn run_incoming_quic(
    svc: Arc<TransferService>,
    connection: quinn::Connection,
) -> Result<()> {
    let t = svc.config.timeout;
    let cert_id = crate::quic::peer_device_id(&connection)?;
    let (send, recv) = connection
        .accept_bi()
        .await
        .map_err(|e| Aa4cError::Network(format!("quic accept stream: {e}")))?;
    // `QuicDuplex` 把 `connection` 和流绑在一起：`IndexRequest` 的分流是转交给 Core 的
    // 钩子后立即返回（钩子内部自己 spawn，不等它跑完），若只传流、让这里的 `connection`
    // 局部变量随本函数返回而丢弃，钩子那个后台任务还没来得及读写就会先撞见连接被关
    // （`Offer`/`FetchRequest` 分支全程 `.await` 到底，不受影响，见 `quic::QuicDuplex` 文档）。
    let mut stream = crate::quic::QuicDuplex::new(connection, recv, send);

    let (hello_id, proto) = server_hello(&mut stream, svc.identity.device_id()).await?;
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

    dispatch_shared(svc, Box::new(stream), cert_id, trusted, proto, first).await
}

/// 中继入站连接入口（里程碑 C3）：`stream` 是已撮合的中继裸管道（`aa4c-core::server_link`
/// 完成 `RelayOpen` 后交给 [`crate::TransferService::accept_external`]），本函数在其上
/// **叠加一次设备间 TLS accept**，随后与 TCP/QUIC 入站完全同构——`dispatch_shared` 分不出
/// 底下换了承载。配对不支持中继（同 QUIC，见 [`dispatch_shared`] 的 `PairRequest` 分支）。
pub(crate) async fn run_incoming_external(
    svc: Arc<TransferService>,
    raw: SharedStream,
) -> Result<()> {
    let t = svc.config.timeout;
    let tls_config = svc.identity.tls_server_config(None)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let mut stream = acceptor
        .accept(raw)
        .await
        .map_err(|e| Aa4cError::Network(format!("relay tls accept: {e}")))?;

    let cert_id = {
        let certs = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| Aa4cError::Protocol("peer presented no certificate".into()))?;
        device_id_from_cert(certs)?
    };
    let (hello_id, proto) = server_hello(&mut stream, svc.identity.device_id()).await?;
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

    dispatch_shared(svc, Box::new(stream), cert_id, trusted, proto, first).await
}

/// 共享分流：入站连接读出首消息为 Offer/IndexRequest/FetchRequest 时的处理，
/// TCP 与 QUIC 通用（里程碑 C1）。`PairRequest` 在此仅作为「不支持」处理——
/// 配对目前只走 [`run_incoming`] 专属路径，不会以这个消息类型进入本函数。
async fn dispatch_shared(
    svc: Arc<TransferService>,
    mut stream: SharedStream,
    cert_id: DeviceId,
    trusted: bool,
    proto: u16,
    first: Message,
) -> Result<()> {
    match first {
        Message::Offer { task_id, files } => {
            if !trusted {
                // PROTOCOL §5 规则 3：未配对设备直接拒绝
                let _ = write_message(
                    &mut stream,
                    &Message::Cancel {
                        task_id: task_id.clone(),
                        reason: "not_paired".into(),
                    },
                )
                .await;
                return Err(Aa4cError::NotPaired(cert_id));
            }
            // 信号在这里登记（而不是 session 内部）：收尾时要拿它判断"我这一轮
            // 是不是已经被『继续』拉起的新一轮顶替了"，见 `finish_task_if_current`。
            let signal = svc.register_cancel(&task_id);
            let result = session(
                &svc,
                &mut stream,
                &cert_id,
                task_id.clone(),
                files,
                proto,
                &signal,
            )
            .await;
            svc.finish_task_if_current(&task_id, &signal, result).await;
            Ok(())
        }
        // 索引交换：仅已配对设备，交给 Core 注入的分流钩子（完全信任过滤在 Core 端）
        Message::IndexRequest => {
            if !trusted {
                return Err(Aa4cError::NotPaired(cert_id));
            }
            match svc.index_dispatch.get() {
                Some(dispatch) => {
                    dispatch.dispatch(stream, cert_id);
                    Ok(())
                }
                None => Err(Aa4cError::Protocol("index dispatch not wired".into())),
            }
        }
        // 引荐：仅**完全信任**设备之间交换（TRUST_DESIGN.md §5.5，PROTOCOL.md §18）。
        // 与 `IndexRequest` 不同，这里**不需要 Core 注入钩子**——应答内容全部来自
        // `store` 里的已配对设备表，没有任何范围 / 路径策略要问 Core，而 TransferService
        // 本来就持有 store。
        Message::IntroduceRequest => {
            if !trusted {
                return Err(Aa4cError::NotPaired(cert_id));
            }
            serve_introductions(&svc, &mut stream, &cert_id).await
        }
        // 按需拉取：仅完全信任设备可拉（边界由 Core 注入的解析器把关）。解析成功后
        // **反转角色**，本端在同一连接上变身发送方回推该文件（里程碑 4）。
        Message::FetchRequest { rel_path } => {
            if !trusted {
                return Err(Aa4cError::NotPaired(cert_id));
            }
            let Some(resolver) = svc.fetch_resolver.get() else {
                return Err(Aa4cError::Protocol("fetch resolver not wired".into()));
            };
            match resolver.resolve(cert_id.clone(), rel_path).await {
                Some(resolved) => {
                    let task_id = uuid::Uuid::new_v4().to_string();
                    let result =
                        crate::send::serve_fetch(&svc, &mut stream, &cert_id, &task_id, resolved)
                            .await;
                    svc.finish_task(&task_id, result).await;
                    Ok(())
                }
                None => {
                    // 不在共享范围 / 非 full：回 Cancel，不泄露存在性细节
                    let _ = write_message(
                        &mut stream,
                        &Message::Cancel {
                            task_id: String::new(),
                            reason: "not_shared".into(),
                        },
                    )
                    .await;
                    Err(Aa4cError::Protocol("fetch denied".into()))
                }
            }
        }
        // 分享链接（里程碑 C6）：**不检查 `trusted`**——token 本身就是访问能力
        // （CONNECT_DESIGN.md §7.1），Core 注入的解析器内部校验 token 有效性 + 路径边界。
        Message::ShareRequest { token } => {
            let Some(resolver) = svc.share_resolver.get() else {
                return Err(Aa4cError::Protocol("share resolver not wired".into()));
            };
            match resolver.resolve(token, cert_id.clone()).await {
                Some(resolved) => {
                    let task_id = uuid::Uuid::new_v4().to_string();
                    let result =
                        crate::send::serve_fetch(&svc, &mut stream, &cert_id, &task_id, resolved)
                            .await;
                    svc.finish_task(&task_id, result).await;
                    Ok(())
                }
                None => {
                    // token 不存在 / 已过期 / 已吊销 / 路径解析失败：统一回 Cancel，不区分
                    // 原因（同 FetchRequest 的「不泄露存在性细节」惯例）。
                    let _ = write_message(
                        &mut stream,
                        &Message::Cancel {
                            task_id: String::new(),
                            reason: "invalid_or_expired_token".into(),
                        },
                    )
                    .await;
                    Err(Aa4cError::Protocol("share denied".into()))
                }
            }
        }
        Message::PairRequest { .. } => Err(Aa4cError::Protocol(
            "pairing is not supported on this transport".into(),
        )),
        other => Err(unexpected(&other)),
    }
}

/// 应答 `IntroduceRequest`：把本机的完全信任设备回送给请求方（TRUST_DESIGN.md §5.5）。
///
/// 三道闸，缺一不可：
/// 1. **请求方必须是 `full`**——与文件索引同一道闸（SYNC_DESIGN §2）。`friend` 拿不到。
/// 2. **只回送 `full` 设备**。`friend` 是「别人的设备」，把它引荐给自己的另一台设备等于
///    替用户扩大信任范围，还顺带泄露社交关系图，不做。
/// 3. **排除请求方自己**（它当然已经认识自己），以及待确认记录（`trusted = 0` 的行不在
///    `list_paired_devices` 里）——引荐只传递**本机用户已经亲自确认过**的信任，不做二次
///    转发，否则一条引荐能顺着设备图无限扩散。
async fn serve_introductions(
    svc: &Arc<TransferService>,
    stream: &mut SharedStream,
    cert_id: &DeviceId,
) -> Result<()> {
    use aa4c_proto::PeerIntro;
    use aa4c_types::TrustLevel;

    let asker_is_full = svc
        .store
        .get_device(cert_id)
        .await?
        .is_some_and(|d| d.trusted && d.trust_level == TrustLevel::Full);
    if !asker_is_full {
        let _ = write_message(
            stream,
            &Message::Cancel {
                task_id: String::new(),
                reason: "not_full_trust".into(),
            },
        )
        .await;
        return Err(Aa4cError::Protocol("introduce denied".into()));
    }

    let peers: Vec<PeerIntro> = svc
        .store
        .list_paired_devices()
        .await?
        .into_iter()
        .filter(|d| d.trust_level == TrustLevel::Full && &d.id != cert_id)
        .map(|d| PeerIntro {
            device_id: d.id,
            public_key: d.public_key,
            name: d.name,
            platform: d.platform.as_str().to_string(),
            server_hint: d.server_hint,
        })
        .collect();

    let mut chunks = peers.chunks(crate::INTRODUCE_BATCH).peekable();
    // 空列表也要回一帧 `last = true`，否则请求方会一直等到超时。
    if chunks.peek().is_none() {
        return write_message(
            stream,
            &Message::IntroducePeers {
                peers: Vec::new(),
                last: true,
            },
        )
        .await;
    }
    while let Some(batch) = chunks.next() {
        write_message(
            stream,
            &Message::IntroducePeers {
                peers: batch.to_vec(),
                last: chunks.peek().is_none(),
            },
        )
        .await?;
    }
    Ok(())
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

/// 这条任务此前是否已经被本机同意过；是则返回当时选定的保存目录，用于「继续」时
/// 跳过第二次确认框。
///
/// **只认"同一条任务的原样重来"**：除了要求此前确实同意过（`save_dir` 非空），还
/// 逐项比对本次 Offer 的文件清单与当时落库的那份是否完全一致（文件数、相对路径、
/// 大小）。少了这一层，一个已配对设备就能拿一个用过的 task_id 配一份**不同的**
/// 文件清单绕过确认——同意过的是"那些文件"，不是"这个编号"。任何对不上就照常
/// 走确认流程，宁可多问一次。
async fn remembered_acceptance(
    svc: &Arc<TransferService>,
    task_id: &TaskId,
    files: &[FileMeta],
) -> Option<PathBuf> {
    let dir = svc.store.accepted_save_dir(task_id).await.ok()??;
    let previous = svc.store.get_task(task_id).await.ok()??;
    if previous.direction != Direction::Recv {
        return None;
    }
    same_manifest(&previous.files, files).then(|| PathBuf::from(dir))
}

/// 本次 Offer 的文件清单是否与此前落库的那份完全一致（顺序、相对路径、大小）。
/// 抽成纯函数是为了能直接测——这是"跳过确认框"唯一的把关处。
fn same_manifest(previous: &[TransferFile], incoming: &[FileMeta]) -> bool {
    previous.len() == incoming.len()
        && previous
            .iter()
            .zip(incoming)
            .all(|(old, new)| old.rel_path == new.rel_path && old.size == new.size)
}

/// 接收会话主循环。`proto` 为握手协商版本，决定是否交换断点续传信息（PROTOCOL.md §13）。
async fn session<S>(
    svc: &Arc<TransferService>,
    stream: &mut S,
    peer_id: &str,
    task_id: TaskId,
    files: Vec<FileMeta>,
    proto: u16,
    cancel: &crate::StopSignal,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
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
    // 「暂停 → 继续」会用同一个 task_id 再发一次 Offer。用户上一轮已经同意过了，
    // 不该被第二次确认框打扰——**必须在 `insert_task` 之前问**，那一步会先删掉
    // 同 id 的旧行（连同记着的 save_dir）再重建。
    let remembered = remembered_acceptance(svc, &task_id, &files).await;

    svc.store.insert_task(&task).await?;

    let save_dir = match remembered {
        Some(dir) => {
            tracing::info!(
                task = %task_id,
                "resuming a transfer this device already accepted; not asking again"
            );
            dir
        }
        None => {
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
            save_dir.unwrap_or_else(|| svc.config.default_save_dir.clone())
        }
    };
    tokio::fs::create_dir_all(&save_dir).await?;
    // 记住这次的同意，供下一次「继续」复用（`insert_task` 刚把这一列清成 NULL，
    // 所以每一轮都要重新写，不能只在首次同意时写）。
    let _ = svc
        .store
        .set_task_save_dir(&task_id, &save_dir.to_string_lossy())
        .await;
    write_message(stream, &answer(&task_id, true)).await?;
    svc.store
        .update_task_status(&task_id, TransferStatus::Transferring, None)
        .await?;

    // 断点续传（PROTOCOL.md §13，里程碑 C1）：双方协商 proto ≥ RESUME_PROTO_VERSION 时
    // **确定性**交换（不是尝试性的）——接收方探测已落盘的 .aa4c-part，回告可信续传起点。
    let resume = if proto >= aa4c_types::RESUME_PROTO_VERSION {
        let progress = resume_progress(&save_dir, &rel_paths, &files).await;
        write_message(
            stream,
            &Message::ResumeReport {
                task_id: task_id.clone(),
                progress: progress.clone(),
            },
        )
        .await?;
        progress
    } else {
        Vec::new()
    };

    receive_files(
        svc, stream, &task_id, &files, &rel_paths, &save_dir, &resume, cancel,
    )
    .await
}

/// 断点续传：把已落盘 `.aa4c-part` 截断到 4 MiB 边界作为「安全前缀」（PROTOCOL.md §13）。
///
/// 不做逐块签名比对——只信任「完整的 4 MiB 块」，丢弃末尾不足一块的余量（可能是上次
/// 传输中途断连造成的半截写入）。这在任何 chunk_size 配置下都安全：最终会在 `FileDone`
/// 时对整个文件重新校验完整哈希，前缀只要不是「假想」而是真被完整写入过就绝对正确。
async fn resume_progress(
    save_dir: &std::path::Path,
    rel_paths: &[PathBuf],
    files: &[FileMeta],
) -> Vec<FileProgress> {
    const BLOCK: u64 = aa4c_types::CHUNK_SIZE as u64;
    let mut out = Vec::new();
    for (idx, rel) in rel_paths.iter().enumerate() {
        let target = save_dir.join(rel);
        let file_name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let part = target.with_file_name(format!("{file_name}.aa4c-part"));
        let Ok(meta) = tokio::fs::metadata(&part).await else {
            continue;
        };
        let size = meta.len().min(files[idx].size);
        let verified = (size / BLOCK) * BLOCK;
        if verified > 0 {
            out.push(FileProgress {
                file_index: idx as u32,
                verified_bytes: verified,
            });
        }
    }
    out
}

/// 接收文件主循环（PROTOCOL.md §7 右列，已与「接受/握手」解耦）：
/// 收 `Chunk`/`FileDone`、回 `FileAck`、`TaskDone` 收尾。入站推送（[`session`]）与
/// 按需拉取（拉取方反向接收，里程碑 4）共用此循环；`stream` 泛型，故两种连接皆可。
///
/// `resume` 为按需续传的起点报告（里程碑 C1；拉取路径固定传空切片，暂不支持续传）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn receive_files<S>(
    svc: &Arc<TransferService>,
    stream: &mut S,
    task_id: &TaskId,
    files: &[FileMeta],
    rel_paths: &[PathBuf],
    save_dir: &std::path::Path,
    resume: &[FileProgress],
    cancel: &crate::StopSignal,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let t = svc.config.timeout;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut progress = Progress::new(
        task_id.clone(),
        svc.events.clone(),
        svc.store.clone(),
        total,
    );

    let resume_map: HashMap<u32, u64> = resume
        .iter()
        .map(|p| (p.file_index, p.verified_bytes))
        .collect();
    // 续传前缀一次性记入进度（后续同一文件的重试不会重复调用此处，见下方 rollback 修正）。
    for p in resume {
        progress
            .add(p.verified_bytes, &files[p.file_index as usize].rel_path)
            .await;
    }

    let mut done = vec![false; files.len()];
    let mut current: Option<OpenFile> = None;
    let mut parts_created: Vec<PathBuf> = Vec::new();
    let max_chunk = svc.config.chunk_size * 2;
    // 是否为「明确取消」（本地用户取消 / 对端主动发 Cancel）：只有这种情况才清理 part
    // 文件（PROTOCOL.md §7 规则 3）。网络掉线、超时、协议错误等**意外**中断保留 part
    // 文件——这正是断点续传的前提（里程碑 C1），下次重新发起时 `resume_progress`
    // 才有东西可续。孤儿 part 文件的过期清理不在本里程碑范围内。
    let mut explicit_cancel = false;

    let result = loop {
        let msg = tokio::select! {
            () = cancel.stopped() => {
                explicit_cancel = true;
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
                let expected_start = resume_map.get(&file_index).copied().unwrap_or(0);
                // 新文件或重传重启（offset 等于该文件的续传起点且无打开文件）
                if current.as_ref().map(|c| c.index) != Some(file_index) {
                    if current.is_some() || offset != expected_start {
                        break Err(Aa4cError::Protocol("out-of-order chunk".into()));
                    }
                    let open = if expected_start > 0 {
                        open_part_resumed(save_dir, &rel_paths[idx], file_index, expected_start)
                            .await
                    } else {
                        open_part(save_dir, &rel_paths[idx], file_index).await
                    };
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
                let expected_start = resume_map.get(&file_index).copied().unwrap_or(0);
                // 没有任何 Chunk 直接收到 FileDone：要么真是空文件，要么整份都已从续传
                // 起点确认（发送方无需再传任何字节）——两种情况统一按 expected_start
                // 判定，取代原来只认「size==0」的窄条件。
                if current.as_ref().map(|c| c.index) != Some(file_index) {
                    if current.is_some() || expected_start != files[idx].size {
                        break Err(Aa4cError::Protocol("FileDone without chunks".into()));
                    }
                    let open = if expected_start > 0 {
                        open_part_resumed(save_dir, &rel_paths[idx], file_index, expected_start)
                            .await
                    } else {
                        open_part(save_dir, &rel_paths[idx], file_index).await
                    };
                    match open {
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
                    // 只回退本次真正重收的部分：续传前缀的 credit（一次性记入，见上文）保留，
                    // 否则同一连接内的重试会反复扣掉本不该扣的已确认字节。
                    progress.rollback(written - expected_start);
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
                explicit_cancel = true;
                break Err(Aa4cError::Network(format!("peer cancelled: {reason}")));
            }
            other => break Err(unexpected(&other)),
        }
    };

    if result.is_err() && explicit_cancel {
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

/// 打开一个已确认前 `verified_bytes` 字节的 part 文件，续传写入（PROTOCOL.md §13，
/// 里程碑 C1）：重新流式读取已验证前缀喂 hasher（重建增量哈希状态，不做逐块签名
/// 比对——安全性来自「这些字节真被完整写入过」，见 [`resume_progress`] 的注释），
/// 不截断文件，从 `verified_bytes` 处继续写。
async fn open_part_resumed(
    save_dir: &std::path::Path,
    rel: &std::path::Path,
    index: u32,
    verified_bytes: u64,
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

    let mut hasher = blake3::Hasher::new();
    let mut reader = tokio::fs::File::open(&part).await?;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut remaining = verified_bytes;
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..want]).await?;
        hasher.update(&buf[..want]);
        remaining -= want as u64;
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false) // 关键：续传要保留已确认前缀，绝不能截断
        .open(&part)
        .await?;
    file.seek(std::io::SeekFrom::Start(verified_bytes)).await?;

    Ok(OpenFile {
        index,
        part,
        target,
        file,
        hasher,
        written: verified_bytes,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(rel_path: &str, size: u64) -> TransferFile {
        TransferFile {
            rel_path: rel_path.into(),
            size,
            hash: None,
            status: FileStatus::Pending,
        }
    }

    fn offered(rel_path: &str, size: u64) -> FileMeta {
        FileMeta {
            rel_path: rel_path.into(),
            size,
        }
    }

    /// 原样重来（「暂停 → 继续」的正常形态）才算认得。
    #[test]
    fn same_manifest_accepts_an_identical_offer() {
        let previous = vec![stored("a.jpg", 10), stored("dir/b.mp4", 20)];
        let incoming = vec![offered("a.jpg", 10), offered("dir/b.mp4", 20)];
        assert!(same_manifest(&previous, &incoming));
    }

    /// 下面几种都必须落回"照常问一次"——用户当初同意的是**那些文件**，
    /// 不是这个任务编号。已配对设备也不该能拿一个用过的 task_id 夹带别的东西。
    #[test]
    fn same_manifest_rejects_anything_that_differs() {
        let previous = vec![stored("a.jpg", 10), stored("dir/b.mp4", 20)];

        // 换了文件名
        assert!(!same_manifest(
            &previous,
            &[offered("a.jpg", 10), offered("dir/evil.exe", 20)]
        ));
        // 同名但大小不同（内容被换掉）
        assert!(!same_manifest(
            &previous,
            &[offered("a.jpg", 10), offered("dir/b.mp4", 999)]
        ));
        // 多塞一个文件
        assert!(!same_manifest(
            &previous,
            &[
                offered("a.jpg", 10),
                offered("dir/b.mp4", 20),
                offered("extra.sh", 1)
            ]
        ));
        // 少一个文件
        assert!(!same_manifest(&previous, &[offered("a.jpg", 10)]));
        // 顺序变了：file_index 是按顺序对应的，换序等于换了对应关系
        assert!(!same_manifest(
            &previous,
            &[offered("dir/b.mp4", 20), offered("a.jpg", 10)]
        ));
        // 空清单
        assert!(!same_manifest(&previous, &[]));
    }
}
