//! AA4C 传输引擎：ATP 文件收发、BLAKE3 校验、取消与重传。
//!
//! 协议规范见 PROTOCOL.md §7，接口契约见 API_DESIGN.md §6。

#![forbid(unsafe_code)]

mod fetch;
mod path;
mod progress;
mod recv;
mod send;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa4c_identity::Identity;
use aa4c_store::Store;
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, DeviceInfo, Direction, FileStatus, Result, TaskId,
    TransferFile, TransferStatus, TransferTask, CHUNK_SIZE, DEFAULT_PORT,
};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

/// 事件发送端（与 aa4c-core 的事件总线同型）。
pub type EventSender = broadcast::Sender<CoreEvent>;

/// 已完成 TLS 握手的入站服务端流（与配对模块同型）。
pub type IncomingTlsStream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;

/// 统一监听器读到 `PairRequest` 后的配对分流钩子。
///
/// 由 aa4c-core 注入 `PairingManager` 适配器，使传输层不感知配对语义
/// （AGENTS.md：服务间低耦合，编排归 Core）。实现内部应自行 spawn 处理。
pub trait IncomingPairDispatch: Send + Sync + 'static {
    /// 接管一条已读出 Hello + `PairRequest` 的入站连接。
    /// `cert_id` 为对端证书指纹，`device`/`public_key` 取自 `PairRequest`。
    fn dispatch(
        &self,
        stream: IncomingTlsStream,
        cert_id: DeviceId,
        device: DeviceInfo,
        public_key: [u8; 32],
    );
}

/// 统一监听器读到 `IndexRequest` 后的索引分流钩子（SYNC_DESIGN.md §3.3，里程碑 3）。
///
/// 与配对钩子同构：传输层只负责把已握手的入站连接交出去，索引语义（完全信任过滤、
/// 共享范围限定路径）全部归 Core。实现内部应自行 spawn 处理并写回 `IndexEntries`。
pub trait IncomingIndexDispatch: Send + Sync + 'static {
    /// 接管一条已读出 Hello + `IndexRequest` 的入站连接；`peer_id` 为对端证书指纹。
    fn dispatch(&self, stream: IncomingTlsStream, peer_id: DeviceId);
}

/// 解析成功的待回推文件（里程碑 4 按需拉取）。
pub struct ResolvedFetch {
    /// 本机绝对路径。
    pub abs: PathBuf,
    /// 回送给拉取方的展示限定路径（原样回声，便于其归并/转绿）。
    pub rel_path: String,
    pub size: u64,
}

/// 共享文件解析器（里程碑 4）：把拉取方请求的限定展示路径解析为本机共享文件。
///
/// 由 Core 注入：完全信任校验 + 「路径必须落在某个共享范围内」的边界都在实现里把关，
/// 传输层只在解析成功后反转角色回推。返回 `None` = 拒绝（传输层回 `Cancel`）。
pub trait SharedFileResolver: Send + Sync + 'static {
    fn resolve(&self, peer_id: DeviceId, rel_path: String) -> ResolveFuture;
}

/// [`SharedFileResolver::resolve`] 的返回（避免引入 async-trait 依赖）。
pub type ResolveFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<ResolvedFetch>> + Send>>;

/// 接收方用户决定：（是否接收，保存目录覆盖）。
type AcceptDecision = (bool, Option<PathBuf>);

#[derive(Clone)]
pub struct TransferConfig {
    /// 分块大小，默认 4 MiB。
    pub chunk_size: usize,
    /// 默认接收目录（Core 注入平台下载目录）。
    pub default_save_dir: PathBuf,
    /// 发送端并发任务上限。
    pub max_concurrent_tasks: usize,
    /// 协议等待超时（PROTOCOL.md §8）。
    pub timeout: Duration,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size: CHUNK_SIZE,
            default_save_dir: std::env::temp_dir().join("AA4C"),
            max_concurrent_tasks: 4,
            timeout: Duration::from_secs(60),
        }
    }
}

pub struct TransferService {
    pub(crate) identity: Arc<Identity>,
    pub(crate) store: Store,
    pub(crate) events: EventSender,
    pub(crate) config: TransferConfig,
    pending_accepts: Mutex<HashMap<TaskId, oneshot::Sender<AcceptDecision>>>,
    cancels: Mutex<HashMap<TaskId, CancellationToken>>,
    send_permits: Arc<Semaphore>,
    /// 配对分流钩子（Core 注入；未注入时入站 `PairRequest` 直接拒绝）。
    pub(crate) pair_dispatch: OnceLock<Arc<dyn IncomingPairDispatch>>,
    /// 索引分流钩子（Core 注入；未注入时入站 `IndexRequest` 直接断开）。
    pub(crate) index_dispatch: OnceLock<Arc<dyn IncomingIndexDispatch>>,
    /// 共享文件解析器（Core 注入；未注入时入站 `FetchRequest` 直接断开）。
    pub(crate) fetch_resolver: OnceLock<Arc<dyn SharedFileResolver>>,
}

impl TransferService {
    pub fn new(
        identity: Arc<Identity>,
        store: Store,
        events: EventSender,
        config: TransferConfig,
    ) -> Arc<Self> {
        let permits = config.max_concurrent_tasks.max(1);
        Arc::new(Self {
            identity,
            store,
            events,
            config,
            pending_accepts: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            send_permits: Arc::new(Semaphore::new(permits)),
            pair_dispatch: OnceLock::new(),
            index_dispatch: OnceLock::new(),
            fetch_resolver: OnceLock::new(),
        })
    }

    /// 注入配对分流钩子（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_pair_dispatch(&self, dispatch: Arc<dyn IncomingPairDispatch>) {
        let _ = self.pair_dispatch.set(dispatch);
    }

    /// 注入索引分流钩子（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_index_dispatch(&self, dispatch: Arc<dyn IncomingIndexDispatch>) {
        let _ = self.index_dispatch.set(dispatch);
    }

    /// 注入共享文件解析器（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_fetch_resolver(&self, resolver: Arc<dyn SharedFileResolver>) {
        let _ = self.fetch_resolver.set(resolver);
    }

    /// 启动 TLS 监听。`port` 被占用时自动向后递增（最多 16 个），返回实际端口。
    pub async fn start_listener(self: &Arc<Self>, port: u16) -> Result<u16> {
        let listener = bind_with_fallback(port).await?;
        let actual = listener.local_addr()?.port();
        let acceptor = TlsAcceptor::from(Arc::new(self.identity.tls_server_config(None)?));
        let svc = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, peer)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let svc = svc.clone();
                tokio::spawn(async move {
                    match acceptor.accept(tcp).await {
                        Ok(tls) => {
                            if let Err(e) = recv::run_incoming(svc, tls).await {
                                tracing::warn!(%peer, error = %e, "incoming session ended with error");
                            }
                        }
                        Err(e) => tracing::debug!(%peer, error = %e, "tls accept failed"),
                    }
                });
            }
        });
        tracing::info!(port = actual, "transfer listener started");
        Ok(actual)
    }

    /// 发起 AA 发送：立即返回 task_id，进度通过事件推送。
    pub async fn send(self: &Arc<Self>, peer: &DeviceInfo, paths: Vec<PathBuf>) -> Result<TaskId> {
        let record = self
            .store
            .get_device(&peer.id)
            .await?
            .filter(|d| d.trusted)
            .ok_or_else(|| Aa4cError::NotPaired(peer.id.clone()))?;
        let addr = peer
            .addr
            .or_else(|| record.last_addr.as_deref().and_then(|s| s.parse().ok()))
            .ok_or_else(|| Aa4cError::DeviceNotFound(peer.id.clone()))?;

        let files = path::build_manifest(&paths).await?;
        let total: u64 = files.iter().map(|f| f.meta.size).sum();
        let task_id = uuid::Uuid::new_v4().to_string();

        self.store
            .insert_task(&TransferTask {
                id: task_id.clone(),
                direction: Direction::Send,
                peer: peer.id.clone(),
                files: files
                    .iter()
                    .map(|f| TransferFile {
                        rel_path: f.meta.rel_path.clone(),
                        size: f.meta.size,
                        hash: None,
                        status: FileStatus::Pending,
                    })
                    .collect(),
                status: TransferStatus::WaitingAccept,
                total_bytes: total,
                transferred_bytes: 0,
                created_at: now_ms(),
                error: None,
            })
            .await?;

        let cancel = self.register_cancel(&task_id);
        let job = send::SendJob {
            task_id: task_id.clone(),
            peer_id: peer.id.clone(),
            addr,
            files,
            total,
        };
        let svc = self.clone();
        let permits = self.send_permits.clone();
        tokio::spawn(async move {
            let _permit = permits.acquire_owned().await;
            send::run(svc, job, cancel).await;
        });
        Ok(task_id)
    }

    /// 向某完全信任设备拉取共享索引（SYNC_DESIGN.md §3.3，里程碑 3）。
    ///
    /// 建连 → 握手（校验证书指纹）→ `IndexRequest` → 分批读 `IndexEntries` 直至 `last`。
    /// 只取元数据、不取内容；调用方（Core）负责落 `remote_index` 并判定黄/红。
    pub async fn fetch_index(
        &self,
        peer_id: &DeviceId,
        addr: std::net::SocketAddr,
    ) -> Result<Vec<aa4c_proto::IndexItem>> {
        use aa4c_proto::{client_hello, read_message, write_message, Message};
        use tokio::net::TcpStream;
        use tokio::time::timeout;
        use tokio_rustls::TlsConnector;

        let t = self.config.timeout;
        let tcp = timeout(t, TcpStream::connect(addr))
            .await
            .map_err(|_| Aa4cError::Network("connect timeout".into()))??;
        let config = self.identity.tls_client_config(Some(peer_id))?;
        let mut stream = TlsConnector::from(Arc::new(config))
            .connect(
                tokio_rustls::rustls::pki_types::ServerName::try_from("aa4c").expect("static name"),
                tcp,
            )
            .await?;

        let (hello_id, _proto) = client_hello(&mut stream, self.identity.device_id()).await?;
        if &hello_id != peer_id {
            return Err(Aa4cError::Protocol("hello id != expected peer".into()));
        }
        write_message(&mut stream, &Message::IndexRequest).await?;

        let mut items = Vec::new();
        // 上限保护：避免对端发送无限批次（每批最多 INDEX_BATCH 条，见 Core serve 端）
        for _ in 0..100_000u32 {
            match timeout(t, read_message(&mut stream))
                .await
                .map_err(|_| Aa4cError::Network("index entries timeout".into()))??
            {
                Message::IndexEntries { entries, last } => {
                    items.extend(entries);
                    if last {
                        return Ok(items);
                    }
                }
                Message::Cancel { reason, .. } => {
                    return Err(Aa4cError::Network(format!("peer refused index: {reason}")));
                }
                other => return Err(aa4c_proto::unexpected(&other)),
            }
        }
        Err(Aa4cError::Protocol("index stream too long".into()))
    }

    /// 向某完全信任设备按需拉取一个共享文件（SYNC_DESIGN.md §4，里程碑 4）。
    ///
    /// 立即返回 A 侧 `task_id`（进度走事件总线）；连接、`FetchRequest`、自动接受、接收落盘
    /// 由后台任务驱动。`rel_path` 为统一视图的限定展示路径；`save_dir` 缺省为 Inbox。
    pub async fn fetch_file(
        self: &Arc<Self>,
        peer: &DeviceInfo,
        rel_path: &str,
        save_dir: Option<PathBuf>,
    ) -> Result<TaskId> {
        let record = self
            .store
            .get_device(&peer.id)
            .await?
            .filter(|d| d.trusted)
            .ok_or_else(|| Aa4cError::NotPaired(peer.id.clone()))?;
        let addr = peer
            .addr
            .or_else(|| record.last_addr.as_deref().and_then(|s| s.parse().ok()))
            .ok_or_else(|| Aa4cError::DeviceNotFound(peer.id.clone()))?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let job = fetch::FetchJob {
            task_id: task_id.clone(),
            peer_id: peer.id.clone(),
            addr,
            rel_path: rel_path.to_string(),
            save_dir,
        };
        let svc = self.clone();
        let permits = self.send_permits.clone();
        tokio::spawn(async move {
            let _permit = permits.acquire_owned().await;
            fetch::run(svc, job).await;
        });
        Ok(task_id)
    }

    /// 接收端用户确认（save_dir 为空使用默认目录）。
    pub async fn accept(
        &self,
        task_id: &TaskId,
        accept: bool,
        save_dir: Option<PathBuf>,
    ) -> Result<()> {
        let tx = self
            .pending_accepts
            .lock()
            .expect("pending lock")
            .remove(task_id)
            .ok_or_else(|| Aa4cError::Protocol(format!("no pending transfer: {task_id}")))?;
        tx.send((accept, save_dir))
            .map_err(|_| Aa4cError::Protocol("transfer session already ended".into()))
    }

    /// 取消任务（双方均可）。
    pub async fn cancel(&self, task_id: &TaskId) -> Result<()> {
        let token = self
            .cancels
            .lock()
            .expect("cancels lock")
            .get(task_id)
            .cloned()
            .ok_or_else(|| Aa4cError::Protocol(format!("unknown task: {task_id}")))?;
        token.cancel();
        Ok(())
    }

    // —— 会话簿记（send/recv 模块共用） ——

    pub(crate) fn register_cancel(&self, task_id: &TaskId) -> CancellationToken {
        let token = CancellationToken::new();
        self.cancels
            .lock()
            .expect("cancels lock")
            .insert(task_id.clone(), token.clone());
        token
    }

    pub(crate) fn register_pending_accept(
        &self,
        task_id: &TaskId,
    ) -> oneshot::Receiver<AcceptDecision> {
        let (tx, rx) = oneshot::channel();
        self.pending_accepts
            .lock()
            .expect("pending lock")
            .insert(task_id.clone(), tx);
        rx
    }

    /// 会话收尾：状态落库 + 事件 + 簿记清理。
    pub(crate) async fn finish_task(&self, task_id: &TaskId, result: Result<()>) {
        self.cancels.lock().expect("cancels lock").remove(task_id);
        self.pending_accepts
            .lock()
            .expect("pending lock")
            .remove(task_id);

        let (status, error) = match &result {
            Ok(()) => (TransferStatus::Done, None),
            Err(Aa4cError::Cancelled) => (TransferStatus::Cancelled, Some("已取消".to_string())),
            Err(Aa4cError::TransferRejected) => (
                TransferStatus::Rejected,
                Some("对方拒绝了这次传输".to_string()),
            ),
            Err(e) => (TransferStatus::Failed, Some(e.to_string())),
        };
        if let Err(e) = self
            .store
            .update_task_status(task_id, status, error.as_deref())
            .await
        {
            tracing::warn!(task = %task_id, error = %e, "failed to persist final status");
        }
        let event = match status {
            TransferStatus::Done => CoreEvent::TransferDone {
                task_id: task_id.clone(),
            },
            _ => CoreEvent::TransferFailed {
                task_id: task_id.clone(),
                error: error.unwrap_or_default(),
            },
        };
        let _ = self.events.send(event);
    }
}

/// 端口占用时自动递增（PROTOCOL.md §1）。
async fn bind_with_fallback(port: u16) -> Result<TcpListener> {
    // 端口 0 = 系统分配（测试用），不做递增
    if port == 0 {
        return Ok(TcpListener::bind(("0.0.0.0", 0)).await?);
    }
    let mut last_err = None;
    for offset in 0..16u16 {
        let candidate = port.checked_add(offset).unwrap_or(DEFAULT_PORT);
        match TcpListener::bind(("0.0.0.0", candidate)).await {
            Ok(l) => return Ok(l),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.expect("at least one attempt").into())
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
