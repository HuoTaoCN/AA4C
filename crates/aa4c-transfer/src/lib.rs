//! AA4C 传输引擎：ATP 文件收发、BLAKE3 校验、取消与重传。
//!
//! 协议规范见 PROTOCOL.md §7，接口契约见 API_DESIGN.md §6。

#![forbid(unsafe_code)]

mod fetch;
mod path;
mod progress;
mod quic;
mod recv;
mod send;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa4c_identity::Identity;
use aa4c_store::Store;
use aa4c_types::{
    Aa4cError, ConnectionVia, CoreEvent, DeviceId, DeviceInfo, Direction, FileStatus, Result,
    TaskId, TransferFile, TransferStatus, TransferTask, CHUNK_SIZE, DEFAULT_PORT,
};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

/// 事件发送端（与 aa4c-core 的事件总线同型）。
pub type EventSender = broadcast::Sender<CoreEvent>;

/// 打洞阶段单个候选地址的连接尝试上限（里程碑 C5）：真正打通的候选几个 RTT 就该有
/// 响应，候选列表可能有好几个，不能让一个不通的候选拖累整条阶梯的失败延迟。
const PUNCH_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(2);

/// 已完成 TLS 握手的入站服务端流（与配对模块同型）。局域网配对目前只走这条具体类型
/// （见 [`IncomingPairDispatch`]）；索引/拉取的入站分流走下面泛化的 [`SharedStream`]，
/// 因为它们还要支持 QUIC 入站连接（里程碑 C1，CONNECT_DESIGN.md §5）。
pub type IncomingTlsStream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;

/// 标记 trait：任何同时实现 `AsyncRead + AsyncWrite`（且 `Unpin + Send`）的双工流都自动满足，
/// 用于把 TCP+TLS 与 QUIC 两种入站/出站连接抹平成同一个可动态分发的类型
/// （里程碑 C1：索引交换/按需拉取要在两种承载层上跑同一套分发逻辑，见 CONNECT_DESIGN.md §5）。
pub trait AsyncDuplex: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncDuplex for T {}

/// 装箱的双工流：索引/拉取分流钩子的入参类型（TCP 或 QUIC 均可，见 [`AsyncDuplex`]）。
/// 配对（[`IncomingPairDispatch`]）范围仍限局域网 TCP，未纳入此抽象（V0.3 未做远程配对）。
pub type SharedStream = Box<dyn AsyncDuplex>;

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
    /// 接管一条已读出 Hello + `IndexRequest` 的入站连接（TCP 或 QUIC）；`peer_id` 为对端证书指纹。
    fn dispatch(&self, stream: SharedStream, peer_id: DeviceId);
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

/// 连接阶梯第 4 档：中继兜底拨号（里程碑 C3，CONNECT_DESIGN.md §2/§4）。
///
/// 由 Core 注入（实现内部会去连自建服务器申请 `RelayRequest`/`RelayOpen`，这些协议细节
/// 都在 `aa4c-core::server_link`，传输层不感知服务器地址/mTLS-vs-Challenge 这些取舍）。
/// 返回的是**中继裸管道**（尚未叠加设备间 mTLS）——[`TransferService::dial`] 收到后会
/// 像对待新拨的 TCP 连接一样在其上再做一次设备间 `TlsConnector::connect`，语义与直连
/// 完全对称（中继只是换了一层承载）。
pub trait RelayDialer: Send + Sync + 'static {
    /// 尝试为 `peer_id` 建一条中继裸管道；不可达/未配置服务器/对端不在线均返回 `Err`
    /// （调用方 [`TransferService::dial`] 会把它当作「这一档也失败了」处理）。
    fn dial(&self, peer_id: DeviceId) -> RelayDialFuture;
}

/// [`RelayDialer::dial`] 的返回（避免引入 async-trait 依赖）。
pub type RelayDialFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<SharedStream>> + Send>>;

/// 连接阶梯第 3 档：NAT 打洞候选交换（里程碑 C5，CONNECT_DESIGN.md §2）。
///
/// 由 Core 注入（实现内部会经自建服务器的常驻连接做 `Signal`/`IncomingSignal` 候选
/// 交换，见 `aa4c-core::server_link`；传输层不感知服务器协议细节，只关心"给我一份
/// 候选地址试试"）。传输层拿到候选后自己用现有 `quic::connect` 逐个尝试——第一个握手
/// 成功的即为打洞直连（`ConnectionVia::Punch`）。实现内部**也会**顺带向 `peer_id`
/// 打几个尽力而为的探测包（帮对方的真实连接尝试捅穿本机 NAT），这部分对调用方透明。
pub trait PunchDialer: Send + Sync + 'static {
    /// 尝试为 `peer_id` 换一份候选地址；交换失败/超时（对端不可达/未维持常驻连接/
    /// 未配置服务器）均返回 `Err`，调用方会把它当作「这一档也失败了」直接落中继。
    fn candidates(&self, peer_id: DeviceId) -> PunchFuture;
}

/// [`PunchDialer::candidates`] 的返回（避免引入 async-trait 依赖）。
pub type PunchFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<std::net::SocketAddr>>> + Send>>;

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
    /// 出站连接优先走 QUIC（里程碑 C1 的测试/联调开关；正式的「按可达性自动选择
    /// 直连/打洞/中继」逻辑收口在里程碑 C4，见 CONNECT_DESIGN.md §11）。默认 `false`
    /// 不影响任何现有行为；仅当为 `true` 且本机 QUIC 监听已就绪时才生效，否则报错
    /// （调用方不应该在没有 QUIC 端点时开启此项）。
    pub prefer_quic: bool,
    /// 跳过连接阶梯第 3 档（打洞，里程碑 C5）的测试/联调开关。默认 `false` 不影响任何
    /// 现有行为；存在的唯一原因是**测试环境没有真实 NAT**——回环地址天然可达，打洞在
    /// 任何测试/CI 环境下都会稳定成功，导致想专门验证「中继兜底」的测试其实从没真的
    /// 走到中继（实测踩到：C3 的 `forced_relay_path_completes_a_transfer` 在 C5 加入
    /// 打洞后悄悄变成了在测打洞，见 CHANGELOG）。开着这个开关能让相关测试确定性地
    /// 绕过打洞，真正逼出第 4 档。
    pub disable_punch: bool,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size: CHUNK_SIZE,
            default_save_dir: std::env::temp_dir().join("AA4C"),
            max_concurrent_tasks: 4,
            timeout: Duration::from_secs(60),
            prefer_quic: false,
            disable_punch: false,
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
    /// QUIC 端点（`start_listener` best-effort 绑定成功后写入；绑定失败则永远是空，
    /// 出站连接自动回落 TCP，见 [`dial`]）。同一端点兼做出站连接，quinn 官方推荐用法。
    pub(crate) quic_endpoint: OnceLock<quinn::Endpoint>,
    /// 中继拨号器（Core 注入；未注入时连接阶梯只到「公网直连」为止，直连失败即报错，
    /// 见 [`dial`]，里程碑 C3）。
    pub(crate) relay_dialer: OnceLock<Arc<dyn RelayDialer>>,
    /// 打洞拨号器（Core 注入；未注入时直接跳过第 3 档落中继，见 [`dial`]，里程碑 C5）。
    pub(crate) punch_dialer: OnceLock<Arc<dyn PunchDialer>>,
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
            quic_endpoint: OnceLock::new(),
            relay_dialer: OnceLock::new(),
            punch_dialer: OnceLock::new(),
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

    /// 注入中继拨号器（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_relay_dialer(&self, dialer: Arc<dyn RelayDialer>) {
        let _ = self.relay_dialer.set(dialer);
    }

    /// 注入打洞拨号器（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_punch_dialer(&self, dialer: Arc<dyn PunchDialer>) {
        let _ = self.punch_dialer.set(dialer);
    }

    /// 反射地址探测（里程碑 C5，连接阶梯第 3 档打洞用）：见 `quic::reflexive_addr`。
    /// 本机 QUIC 端点未就绪时直接报错——没有 QUIC 就没有打洞可言，同中继档「没有拨号器
    /// 就报错」的降级逻辑一致。
    pub async fn reflexive_addr(
        &self,
        reflect_addr: std::net::SocketAddr,
        expected_fingerprint_prefix: &str,
    ) -> Result<std::net::SocketAddr> {
        let endpoint = self.quic_endpoint.get().ok_or_else(|| {
            Aa4cError::Network("quic endpoint not available, cannot probe reflexive address".into())
        })?;
        quic::reflexive_addr(
            endpoint,
            &self.identity,
            reflect_addr,
            expected_fingerprint_prefix,
        )
        .await
    }

    /// 向 `peer_id` 打几个尽力而为的探测包（帮对方的真实连接尝试捅穿本机 NAT，里程碑
    /// C5）：内部就是对每个候选地址发起一次 `quic::connect`，**完全不关心结果**——
    /// 无论成败，NAT 映射该开的口子已经在发包那一刻开了，success/failure 只是这条
    /// 连接本身要不要保留的问题，我们不需要保留它。本机没有 QUIC 端点时静默跳过。
    pub fn punch_probe(self: &Arc<Self>, peer_id: DeviceId, candidates: Vec<std::net::SocketAddr>) {
        let Some(endpoint) = self.quic_endpoint.get().cloned() else {
            return;
        };
        let identity = self.identity.clone();
        for addr in candidates {
            let endpoint = endpoint.clone();
            let identity = identity.clone();
            let peer_id = peer_id.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    quic::connect(&endpoint, &identity, &peer_id, addr),
                )
                .await;
            });
        }
    }

    /// 接管一条已就绪的外部入站裸管道（目前仅用于中继数据面，里程碑 C3）：在其上
    /// 叠加一次设备间 TLS accept，再走与 TCP/QUIC 入站完全相同的握手 + 分流
    /// （[`recv::run_incoming_external`]）。Core 的 `server_link` 在收到服务器推送的
    /// `IncomingRelay` 并完成 `RelayOpen` 撮合后调用本方法。
    pub fn accept_external(self: &Arc<Self>, stream: SharedStream) {
        let svc = self.clone();
        tokio::spawn(async move {
            if let Err(e) = recv::run_incoming_external(svc, stream).await {
                tracing::warn!(error = %e, "incoming relay session ended with error");
            }
        });
    }

    /// 启动 TLS 监听。`port` 被占用时自动向后递增（最多 16 个），返回实际端口。
    ///
    /// 同时 best-effort 绑定 QUIC（UDP 同一端口号，见 CONNECT_DESIGN.md §5）：绑不上
    /// 只警告，不阻断启动——没有 QUIC 就回落纯局域网 TCP 行为，不影响 V0.1/V0.2 既有能力。
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

        match quic::listen(self.clone(), &self.identity, actual) {
            Ok(endpoint) => {
                let _ = self.quic_endpoint.set(endpoint);
                tracing::info!(port = actual, "quic listener started");
            }
            Err(e) => {
                tracing::warn!(error = %e, port = actual, "quic listener unavailable, WAN transport disabled");
            }
        }

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
            .or_else(|| record.last_addr.as_deref().and_then(|s| s.parse().ok()));
        // 没有任何直连地址：还有中继兜底（里程碑 C3）才继续，否则和以前一样直接报错
        // （连接阶梯第 4 档不需要预先解析地址，但至少要有个能问的服务器，见 `RelayDialer`）。
        if addr.is_none() && self.relay_dialer.get().is_none() {
            return Err(Aa4cError::DeviceNotFound(peer.id.clone()));
        }

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

    /// 出站连接：按配置选 QUIC 或 TCP+TLS，直连失败（或压根没有地址）时落到中继兜底，
    /// 抹平成同一个装箱双工流（里程碑 C1 QUIC/TCP，里程碑 C3 中继）。
    ///
    /// `prefer_quic=false`（默认）：走既有 TCP+TLS 路径，与 V0.1/V0.2 完全一致、零回归。
    /// `prefer_quic=true`：要求本机 QUIC 端点已就绪且已解析出地址，否则报错，不落中继——
    /// 这是里程碑 C1 的测试/联调专用开关，不参与连接阶梯的自动降级。
    ///
    /// `addr` 为 `None`（本档、直连都没解析出地址）时直接尝试中继；`Some` 但连接失败时
    /// 也会尝试中继（只要 Core 注入了 [`RelayDialer`]）——这正是连接阶梯第 4 档
    /// （CONNECT_DESIGN.md §2）：没配置服务器/没有中继拨号器时原样报错，行为与 V0.2 一致。
    ///
    /// 返回值额外带上实际走的档位（`ConnectionVia`，里程碑 C4 连接质量）：局域网直连、
    /// 公网直连、`prefer_quic` 强制的 QUIC 直连都算 `Direct`（上层/UI 不关心底层承载
    /// 是 TCP 还是 QUIC，只关心「直连」还是「中继」），只有落到 [`dial_via_relay`] 才是
    /// `Relay`。
    pub(crate) async fn dial(
        &self,
        peer_id: &DeviceId,
        addr: Option<std::net::SocketAddr>,
    ) -> Result<(SharedStream, ConnectionVia)> {
        if self.config.prefer_quic {
            let addr = addr.ok_or_else(|| {
                Aa4cError::Network("prefer_quic set but no address resolved".into())
            })?;
            let endpoint = self.quic_endpoint.get().ok_or_else(|| {
                Aa4cError::Network("prefer_quic set but quic endpoint not available".into())
            })?;
            let stream = quic::connect(endpoint, &self.identity, peer_id, addr).await?;
            return Ok((Box::new(stream), ConnectionVia::Direct));
        }

        if let Some(addr) = addr {
            match self.dial_tcp(peer_id, addr).await {
                Ok(stream) => return Ok((stream, ConnectionVia::Direct)),
                Err(e) => {
                    if self.punch_dialer.get().is_none() && self.relay_dialer.get().is_none() {
                        return Err(e);
                    }
                    tracing::debug!(peer = %peer_id, error = %e, "direct dial failed, falling back to punch/relay");
                }
            }
        }

        // 连接阶梯第 3 档：NAT 打洞（里程碑 C5）。没有 QUIC 端点就没有打洞可言
        // （反射地址/候选连接都要用它），直接跳过落中继，同 `prefer_quic` 分支的降级逻辑；
        // `disable_punch` 是测试/联调专用开关，见其文档。
        if !self.config.disable_punch {
            if let (Some(punch), Some(endpoint)) =
                (self.punch_dialer.get(), self.quic_endpoint.get())
            {
                match punch.candidates(peer_id.clone()).await {
                    Ok(candidates) => {
                        for candidate in candidates {
                            let attempt = tokio::time::timeout(
                                PUNCH_CANDIDATE_TIMEOUT,
                                quic::connect(endpoint, &self.identity, peer_id, candidate),
                            )
                            .await;
                            if let Ok(Ok(stream)) = attempt {
                                return Ok((Box::new(stream), ConnectionVia::Punch));
                            }
                        }
                        tracing::debug!(peer = %peer_id, "punch candidates exchanged but none connected, falling back to relay");
                    }
                    Err(e) => {
                        tracing::debug!(peer = %peer_id, error = %e, "punch candidate exchange failed, falling back to relay");
                    }
                }
            }
        }

        let dialer = self.relay_dialer.get().ok_or_else(|| {
            Aa4cError::Network("no reachable address and no relay configured".into())
        })?;
        let stream = self.dial_via_relay(dialer.as_ref(), peer_id).await?;
        Ok((stream, ConnectionVia::Relay))
    }

    async fn dial_tcp(
        &self,
        peer_id: &DeviceId,
        addr: std::net::SocketAddr,
    ) -> Result<SharedStream> {
        use tokio::net::TcpStream;
        use tokio::time::timeout;
        use tokio_rustls::TlsConnector;

        let t = self.config.timeout;
        let tcp = timeout(t, TcpStream::connect(addr))
            .await
            .map_err(|_| Aa4cError::Network("connect timeout".into()))??;
        let config = self.identity.tls_client_config(Some(peer_id))?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(
                tokio_rustls::rustls::pki_types::ServerName::try_from("aa4c").expect("static name"),
                tcp,
            )
            .await?;
        Ok(Box::new(stream))
    }

    /// 连接阶梯第 4 档：中继拨号器只给一条裸管道，设备间 mTLS 仍在这里叠加——与直连
    /// 路径完全对称（[`dial_tcp`]），对端感知不到底下换了承载（里程碑 C3）。
    async fn dial_via_relay(
        &self,
        dialer: &dyn RelayDialer,
        peer_id: &DeviceId,
    ) -> Result<SharedStream> {
        use tokio_rustls::TlsConnector;

        let raw = dialer.dial(peer_id.clone()).await?;
        let config = self.identity.tls_client_config(Some(peer_id))?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(
                tokio_rustls::rustls::pki_types::ServerName::try_from("aa4c").expect("static name"),
                raw,
            )
            .await?;
        Ok(Box::new(stream))
    }

    /// 向某完全信任设备拉取共享索引（SYNC_DESIGN.md §3.3，里程碑 3；里程碑 C4 起接入
    /// 完整连接阶梯）。
    ///
    /// 建连 → 握手（校验证书指纹）→ `IndexRequest` → 分批读 `IndexEntries` 直至 `last`。
    /// 只取元数据、不取内容；调用方（Core）负责落 `remote_index` 并判定黄/红。
    ///
    /// `addr` 为 `None`（mDNS/落库地址都没解析出来）时直接尝试中继兜底，与 [`Self::send`]
    /// 同一套语义（见 [`Self::dial`]）——索引交换本身不是用户直接发起的「任务」，不需要
    /// 上报连接质量事件，调用方按需丢弃 `dial` 返回的 `ConnectionVia`。
    pub async fn fetch_index(
        &self,
        peer_id: &DeviceId,
        addr: Option<std::net::SocketAddr>,
    ) -> Result<Vec<aa4c_proto::IndexItem>> {
        use aa4c_proto::{client_hello, read_message, write_message, Message};
        use tokio::time::timeout;

        let t = self.config.timeout;
        let (mut stream, _via) = self.dial(peer_id, addr).await?;

        let (hello_id, proto) = client_hello(&mut stream, self.identity.device_id()).await?;
        if &hello_id != peer_id {
            return Err(Aa4cError::Protocol("hello id != expected peer".into()));
        }
        // 版本门槛：对端为 v1（proto<2）不认识索引消息，直接不发（优雅降级，不发 v2 帧）
        if proto < aa4c_types::SYNC_PROTO_VERSION {
            return Err(Aa4cError::Protocol(format!(
                "peer proto {proto} too old for index exchange"
            )));
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
            .or_else(|| record.last_addr.as_deref().and_then(|s| s.parse().ok()));
        // 没有任何直连地址：还有中继兜底（里程碑 C4）才继续，否则和以前一样直接报错
        // （同 `send()` 的判断，见那边注释）。
        if addr.is_none() && self.relay_dialer.get().is_none() {
            return Err(Aa4cError::DeviceNotFound(peer.id.clone()));
        }

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
