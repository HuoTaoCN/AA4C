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
use aa4c_proto::net;
use aa4c_store::Store;
use aa4c_types::{
    Aa4cError, ConnectionVia, CoreEvent, DeviceId, DeviceInfo, Direction, FileStatus, Result,
    TaskId, TransferFile, TransferStatus, TransferTask, CHUNK_SIZE, DEFAULT_PORT,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, Semaphore};
use tokio_rustls::TlsAcceptor;

use tokio_util::sync::CancellationToken;

/// 事件发送端（与 aa4c-core 的事件总线同型）。
pub type EventSender = broadcast::Sender<CoreEvent>;

/// 打洞阶段单个候选地址的连接尝试上限（里程碑 C5）：真正打通的候选几个 RTT 就该有
/// 响应，候选列表可能有好几个，不能让一个不通的候选拖累整条阶梯的失败延迟。
const PUNCH_CANDIDATE_TIMEOUT: Duration = Duration::from_secs(2);

/// 引荐一批的条数（TRUST_DESIGN.md §5.5，里程碑 R2）。「我的几台设备」量级极小，
/// 一批基本就发完了；分批只是为了与 `IndexEntries` 同形、留出扩展余量。
pub(crate) const INTRODUCE_BATCH: usize = 200;

/// 单次引荐交换接受的条数上限与批次上限（TRUST_DESIGN.md §5.9「拒绝服务」）。
/// 一台被攻破的完全信任设备能做的事里，刷海量引荐把本机待确认列表灌爆是最廉价的一种，
/// 这里在读取侧直接截断。
const MAX_INTRODUCE_PEERS: usize = 500;
const MAX_INTRODUCE_BATCHES: u32 = 16;

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
    /// `cert_id` 为对端证书指纹，`device`/`public_key` 取自 `PairRequest`，`proto` 为
    /// Hello 握手已经协商出的 `min(双方)` 版本（`server_hello` 的返回值，供
    /// `PairServerHint` 交换的 gate 判断用，PROTOCOL.md §17）。
    fn dispatch(
        &self,
        stream: IncomingTlsStream,
        cert_id: DeviceId,
        device: DeviceInfo,
        public_key: [u8; 32],
        proto: u16,
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

/// 分享 token 解析器（CONNECT_DESIGN.md §7，里程碑 C6）：把打开分享链接的一方带来的
/// token 解析为本机共享文件。
///
/// 与 [`SharedFileResolver`] 同构，但鉴权依据不同——**不检查 `trusted`**（token 本身就是
/// 访问能力，见 CONNECT_DESIGN.md §7.1），完全信任校验换成了「token 有效（未过期/未吊销）
/// 且解析到的路径落在共享范围内」。由 Core 注入：token 表查询、`share_access` 记账都在
/// 实现内部完成，传输层只在解析成功后反转角色回推。返回 `None` = 拒绝（传输层回 `Cancel`）。
pub trait ShareResolver: Send + Sync + 'static {
    /// `peer_id` 为请求方证书指纹（记访问日志用；token 校验本身不依赖它）。
    fn resolve(&self, token: String, peer_id: DeviceId) -> ResolveFuture;
}

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

/// 「停止当前会话」的信号，外加一位区分**为什么**停：取消还是暂停。
///
/// 两者对协议的要求刚好相反（PROTOCOL.md §7 规则 3）：
/// - **取消**要给对端发一条 `Cancel`，让它清理已落盘的 `.aa4c-part`；
/// - **暂停**恰恰**不能**发——静默断开才会让接收端保留 part 文件，「继续」时
///   才有东西可续（这正是断点续传赖以成立的既有行为，里程碑 C1）。
///
/// 所以不能只用一个裸 `CancellationToken`：发送侧在被叫停的那一刻必须能问出
/// "这是暂停吗"来决定发不发那条 `Cancel`。
#[derive(Clone)]
pub(crate) struct StopSignal {
    token: CancellationToken,
    paused: Arc<AtomicBool>,
}

impl StopSignal {
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 取消：会给对端发 `Cancel`，对端清理 part 文件。
    fn cancel(&self) {
        self.token.cancel();
    }

    /// 暂停：先立起标志位再叫停，顺序要紧——发送循环醒来后要能读到已经是
    /// `true`，否则会走成取消、把对端的 part 文件清掉，续传就没了。
    fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.token.is_cancelled()
    }

    /// 等到被叫停为止（接收端的 select! 分支用）。接收端不区分暂停/取消——
    /// 「暂停」是发送方单方面的动作，对接收端而言就是对端不说话了。
    pub(crate) async fn stopped(&self) {
        self.token.cancelled().await;
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// 是不是同一轮会话发出来的信号（`clone` 出来的算同一轮，`new()` 出来的不算）。
    /// 用于识别"被新一轮顶替掉的旧会话"，见 `finish_task_if_current`。
    fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.paused, &other.paused)
    }

    /// 协议级测试直接驱动 `transfer_files` 时用（那层不经过 `TransferService`）。
    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::new()
    }
}

/// 「继续」一条已暂停的发送任务所需的全部信息。只存在内存里——进程退出即丢失，
/// 重启后那些任务由 `Store::fail_incomplete_tasks` 统一标失败（见那里的注释）。
#[derive(Clone)]
struct CachedSendJob {
    peer_id: DeviceId,
    addr: Option<std::net::SocketAddr>,
    files: Vec<path::SendFile>,
    total: u64,
}

pub struct TransferService {
    pub(crate) identity: Arc<Identity>,
    pub(crate) store: Store,
    pub(crate) events: EventSender,
    pub(crate) config: TransferConfig,
    pending_accepts: Mutex<HashMap<TaskId, oneshot::Sender<AcceptDecision>>>,
    cancels: Mutex<HashMap<TaskId, StopSignal>>,
    /// 发送任务的原始清单，供「继续」时用同一个 task_id 重新发起。只在 `send()`
    /// 里写入（接收/拉取方向不需要——那两个方向的「继续」是对端的事）。
    send_jobs: Mutex<HashMap<TaskId, CachedSendJob>>,
    send_permits: Arc<Semaphore>,
    /// 配对分流钩子（Core 注入；未注入时入站 `PairRequest` 直接拒绝）。
    pub(crate) pair_dispatch: OnceLock<Arc<dyn IncomingPairDispatch>>,
    /// 索引分流钩子（Core 注入；未注入时入站 `IndexRequest` 直接断开）。
    pub(crate) index_dispatch: OnceLock<Arc<dyn IncomingIndexDispatch>>,
    /// 共享文件解析器（Core 注入；未注入时入站 `FetchRequest` 直接断开）。
    pub(crate) fetch_resolver: OnceLock<Arc<dyn SharedFileResolver>>,
    /// 分享 token 解析器（Core 注入；未注入时入站 `ShareRequest` 直接断开，里程碑 C6）。
    pub(crate) share_resolver: OnceLock<Arc<dyn ShareResolver>>,
    /// QUIC 端点（`start_listener` best-effort 绑定成功后写入；绑定失败则永远是空，
    /// 出站连接自动回落 TCP，见 [`dial`]）。同一端点兼做出站连接，quinn 官方推荐用法。
    pub(crate) quic_endpoint: OnceLock<quinn::Endpoint>,
    /// 中继拨号器（Core 注入；未注入时连接阶梯只到「公网直连」为止，直连失败即报错，
    /// 见 [`dial`]，里程碑 C3）。
    pub(crate) relay_dialer: OnceLock<Arc<dyn RelayDialer>>,
    /// 打洞拨号器（Core 注入；未注入时直接跳过第 3 档落中继，见 [`dial`]，里程碑 C5）。
    pub(crate) punch_dialer: OnceLock<Arc<dyn PunchDialer>>,
    /// 停机信号：[`Self::shutdown`] 触发后 accept 循环退出、监听端口释放。
    ///
    /// 此前 accept 循环是个没有出口的 `loop`，`Core::shutdown()` 停了 discovery 却停不了
    /// 它——进程内每起一个 `TransferService` 就永久多占一个监听端口和一条常驻任务。
    shutdown: CancellationToken,
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
            send_jobs: Mutex::new(HashMap::new()),
            send_permits: Arc::new(Semaphore::new(permits)),
            pair_dispatch: OnceLock::new(),
            index_dispatch: OnceLock::new(),
            fetch_resolver: OnceLock::new(),
            share_resolver: OnceLock::new(),
            quic_endpoint: OnceLock::new(),
            relay_dialer: OnceLock::new(),
            punch_dialer: OnceLock::new(),
            shutdown: CancellationToken::new(),
        })
    }

    /// 停止监听：accept 循环退出、TCP 端口与 QUIC 端点一并释放。
    ///
    /// 进行中的会话不强杀（各自的 `StopSignal` 是另一套机制），只是不再接受新连接。
    /// 幂等：重复调用无副作用。
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(endpoint) = self.quic_endpoint.get() {
            // 让 quinn 的 accept 循环拿到 None 自然退出（见 quic::listen）。
            endpoint.close(0u32.into(), b"shutdown");
        }
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

    /// 注入分享 token 解析器（Core 在装配阶段调用一次）。重复设置无效。
    pub fn set_share_resolver(&self, resolver: Arc<dyn ShareResolver>) {
        let _ = self.share_resolver.set(resolver);
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
        let stop = self.shutdown.clone();
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    r = listener.accept() => r,
                };
                let Ok((tcp, peer)) = accepted else {
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
        // 缓存原始清单供「继续」复用（见 `CachedSendJob`）。只有发送方向缓存——
        // 接收方向的「继续」得由对端重新发起，本机说了不算。
        self.send_jobs.lock().expect("jobs lock").insert(
            task_id.clone(),
            CachedSendJob {
                peer_id: peer.id.clone(),
                addr,
                files: files.clone(),
                total,
            },
        );
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
                    tracing::debug!(peer = %peer_id, %addr, error = %e, "direct dial failed, falling back to punch/relay");
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

    /// 向某完全信任设备拉取「它认为也属于同一个人」的设备（TRUST_DESIGN.md §5.5，
    /// PROTOCOL.md §18，里程碑 R2）。
    ///
    /// 与 [`Self::fetch_index`] 完全同形：建连 → 握手（校验证书指纹）→ `IntroduceRequest`
    /// → 分批读 `IntroducePeers` 直至 `last`。只取指纹与公钥，**不产生任何信任**——落库
    /// 成「待确认」由调用方（Core）负责，升级信任必须经由用户点确认。
    ///
    /// 对端 proto < [`aa4c_types::INTRODUCE_PROTO_VERSION`] 时返回错误，调用方跳过这一轮
    /// 即可（索引交换照常，优雅降级）。
    pub async fn fetch_introductions(
        &self,
        peer_id: &DeviceId,
        addr: Option<std::net::SocketAddr>,
    ) -> Result<Vec<aa4c_proto::PeerIntro>> {
        use aa4c_proto::{client_hello, read_message, write_message, Message};
        use tokio::time::timeout;

        let t = self.config.timeout;
        let (mut stream, _via) = self.dial(peer_id, addr).await?;

        let (hello_id, proto) = client_hello(&mut stream, self.identity.device_id()).await?;
        if &hello_id != peer_id {
            return Err(Aa4cError::Protocol("hello id != expected peer".into()));
        }
        if proto < aa4c_types::INTRODUCE_PROTO_VERSION {
            return Err(Aa4cError::Protocol(format!(
                "peer proto {proto} too old for introductions"
            )));
        }
        write_message(&mut stream, &Message::IntroduceRequest).await?;

        let mut peers = Vec::new();
        for _ in 0..MAX_INTRODUCE_BATCHES {
            match timeout(t, read_message(&mut stream))
                .await
                .map_err(|_| Aa4cError::Network("introduce timeout".into()))??
            {
                Message::IntroducePeers {
                    peers: batch, last, ..
                } => {
                    peers.extend(batch);
                    // 拒绝服务防护（TRUST_DESIGN.md §5.9）：恶意 full 设备可以刷海量引荐，
                    // 这里直接截断——引荐条数天然很小（「我的几台设备」）。
                    if peers.len() > MAX_INTRODUCE_PEERS {
                        return Err(Aa4cError::Protocol("too many introductions".into()));
                    }
                    if last {
                        return Ok(peers);
                    }
                }
                Message::Cancel { reason, .. } => {
                    return Err(Aa4cError::Network(format!(
                        "peer refused introductions: {reason}"
                    )));
                }
                other => return Err(aa4c_proto::unexpected(&other)),
            }
        }
        Err(Aa4cError::Protocol("introduce stream too long".into()))
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
            target: fetch::FetchTarget::Path(rel_path.to_string()),
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

    /// 打开一个分享链接：向 `host_id` 请求 `token` 对应的内容（CONNECT_DESIGN.md §7，
    /// 里程碑 C6）。**不要求本机与 `host_id` 已配对**——token 本身就是访问能力，
    /// 与 [`Self::fetch_file`] 唯一的本质区别就是不做 `store.get_device(...).trusted`
    /// 校验；地址解析、连接阶梯、落盘/自动接受流程完全复用同一套 [`fetch`] 模块逻辑。
    pub async fn open_share(
        self: &Arc<Self>,
        host_id: &DeviceId,
        addr: Option<std::net::SocketAddr>,
        token: String,
        save_dir: Option<PathBuf>,
    ) -> Result<TaskId> {
        // 没有任何直连地址：还有中继兜底才继续，否则直接报错（同 `send()`/`fetch_file()`
        // 的判断，见那边注释——两个拨号器目前总是一起注入，只查 relay_dialer 足够）。
        if addr.is_none() && self.relay_dialer.get().is_none() {
            return Err(Aa4cError::DeviceNotFound(host_id.clone()));
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let job = fetch::FetchJob {
            task_id: task_id.clone(),
            peer_id: host_id.clone(),
            addr,
            target: fetch::FetchTarget::Share(token),
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
    ///
    /// 已暂停的任务**没有在跑的会话**（信号在暂停收尾时就清掉了，那一轮确实结束了），
    /// 但用户显然还能取消它——界面上暂停态的卡片一直摆着 ✕。所以这里分两条路：
    /// 有活会话就叫停它，没有则看看是不是"已暂停"，是就地落终态。
    pub async fn cancel(&self, task_id: &TaskId) -> Result<()> {
        let live = self
            .cancels
            .lock()
            .expect("cancels lock")
            .get(task_id)
            .cloned();
        match live {
            Some(signal) => signal.cancel(),
            None => {
                let status = self.store.get_task(task_id).await?.map(|t| t.status);
                if status != Some(TransferStatus::Paused) {
                    return Err(Aa4cError::Protocol(format!("unknown task: {task_id}")));
                }
                // 收尾在这里就地做完：没有会话可以跑 `finish_task`。接收端那边留下的
                // `.aa4c-part` 收不到 `Cancel`（连接早断了）会继续躺着，属于既有的
                // 孤儿 part 文件问题（见 `recv::receive_files` 的注释），不在此处理。
                self.store
                    .update_task_status(task_id, TransferStatus::Cancelled, Some("已取消"))
                    .await?;
                let _ = self.events.send(CoreEvent::TransferFailed {
                    task_id: task_id.clone(),
                    error: "已取消".to_string(),
                });
            }
        }
        // 取消是终态，缓存的发送清单不会再用到。
        self.send_jobs.lock().expect("jobs lock").remove(task_id);
        Ok(())
    }

    /// 暂停一条**发送中**的任务（打磨计划第二步）。
    ///
    /// 与 [`Self::cancel`] 只差"发不发 `Cancel`"这一点，但结果天差地别：暂停走静默
    /// 断开，接收端保留 `.aa4c-part`，[`Self::resume`] 才接得上（详见 [`StopSignal`]）。
    ///
    /// 只对本机发起的发送任务有意义——接收方向没有"我这边继续"的说法（要由发送方
    /// 重新发起），所以没缓存发送清单的任务直接拒绝，而不是假装暂停成功。
    pub async fn pause(&self, task_id: &TaskId) -> Result<()> {
        if !self
            .send_jobs
            .lock()
            .expect("jobs lock")
            .contains_key(task_id)
        {
            return Err(Aa4cError::Protocol(format!(
                "only outgoing transfers can be paused: {task_id}"
            )));
        }
        let signal = self.stop_signal(task_id)?;
        signal.pause();
        Ok(())
    }

    /// 继续一条已暂停的发送任务：**沿用同一个 `task_id`** 重新发起。
    ///
    /// 沿用而不是新建，是为了让接收端的 `resume_progress` 认出这是同一次传输、
    /// 把已落盘的 `.aa4c-part` 接着写（PROTOCOL.md §13）；对用户来说也仍是列表里
    /// 那一条任务，不会莫名多出一条。
    pub async fn resume(self: &Arc<Self>, task_id: &TaskId) -> Result<()> {
        let job = self
            .send_jobs
            .lock()
            .expect("jobs lock")
            .get(task_id)
            .cloned()
            .ok_or_else(|| {
                // 进程重启过：内存里的清单没了（那些任务已被 `fail_incomplete_tasks`
                // 标失败）。给一句人话，别让调用方看到一个裸的 "unknown task"。
                Aa4cError::Protocol(format!("this transfer can no longer be resumed: {task_id}"))
            })?;

        let current = self.store.get_task(task_id).await?;
        match current.map(|t| t.status) {
            Some(TransferStatus::Paused) => {}
            Some(other) => {
                return Err(Aa4cError::Protocol(format!(
                    "transfer is not paused (status: {})",
                    other.as_str()
                )))
            }
            None => return Err(Aa4cError::Protocol(format!("unknown task: {task_id}"))),
        }

        self.store
            .update_task_status(task_id, TransferStatus::WaitingAccept, None)
            .await?;

        // 全新的 StopSignal：上一个已经处于 cancelled 状态，复用会让新会话一起步
        // 就被判定为"已叫停"。
        let signal = self.register_cancel(task_id);
        let send_job = send::SendJob {
            task_id: task_id.clone(),
            peer_id: job.peer_id,
            addr: job.addr,
            files: job.files,
            total: job.total,
        };
        let svc = self.clone();
        let permits = self.send_permits.clone();
        tokio::spawn(async move {
            let _permit = permits.acquire_owned().await;
            send::run(svc, send_job, signal).await;
        });
        Ok(())
    }

    fn stop_signal(&self, task_id: &TaskId) -> Result<StopSignal> {
        self.cancels
            .lock()
            .expect("cancels lock")
            .get(task_id)
            .cloned()
            .ok_or_else(|| Aa4cError::Protocol(format!("unknown task: {task_id}")))
    }

    // —— 会话簿记（send/recv 模块共用） ——

    pub(crate) fn register_cancel(&self, task_id: &TaskId) -> StopSignal {
        let signal = StopSignal::new();
        self.cancels
            .lock()
            .expect("cancels lock")
            .insert(task_id.clone(), signal.clone());
        signal
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

    /// 暂停收尾：与 [`Self::finish_task`] 的关键差别是**保留** `send_jobs` 里的
    /// 清单（[`Self::resume`] 要靠它重新发起），只清掉本次会话的信号与待确认簿记。
    pub(crate) async fn finish_paused_task(&self, task_id: &TaskId) {
        self.cancels.lock().expect("cancels lock").remove(task_id);
        self.pending_accepts
            .lock()
            .expect("pending lock")
            .remove(task_id);

        if let Err(e) = self
            .store
            .update_task_status(task_id, TransferStatus::Paused, None)
            .await
        {
            tracing::warn!(task = %task_id, error = %e, "failed to persist paused status");
        }
        let _ = self.events.send(CoreEvent::TransferPaused {
            task_id: task_id.clone(),
        });
    }

    /// 同 [`Self::finish_task`]，但会先确认**这一轮会话仍然是当前那一轮**。
    ///
    /// 「暂停 → 继续」沿用同一个 `task_id`，于是接收端可能同时存在两轮会话：被暂停
    /// 的那一轮（发送方已静默断开，它还要一小会儿才察觉 EOF）和「继续」拉起的新
    /// 一轮。旧那轮察觉后会走收尾——如果照常执行，它会把任务状态改成 failed、并把
    /// 新那轮的待确认/信号簿记一并删掉，真的把刚接上的传输弄断。实测这个竞态确实
    /// 会发生（集成测试 5 次里稳定复现 1 次）。
    ///
    /// 判定靠"当前登记的信号还是不是我这一轮的"——新会话 `register_cancel` 时会覆盖
    /// 掉 map 里的旧条目，指针一比即知。被顶替的那轮直接静默退出，什么都不碰。
    pub(crate) async fn finish_task_if_current(
        &self,
        task_id: &TaskId,
        signal: &StopSignal,
        result: Result<()>,
    ) {
        let superseded = {
            let guard = self.cancels.lock().expect("cancels lock");
            match guard.get(task_id) {
                Some(current) => !current.same_session(signal),
                // 已经被某一轮收尾清掉了：那一轮才是最终的，这里不再重复写。
                None => true,
            }
        };
        if superseded {
            tracing::debug!(
                task = %task_id,
                "ignoring cleanup from a superseded receive session (task was resumed)"
            );
            return;
        }
        self.finish_task(task_id, result).await;
    }

    /// 会话收尾：状态落库 + 事件 + 簿记清理。
    pub(crate) async fn finish_task(&self, task_id: &TaskId, result: Result<()>) {
        self.cancels.lock().expect("cancels lock").remove(task_id);
        self.send_jobs.lock().expect("jobs lock").remove(task_id);
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
///
/// 绑的是**双栈** `[::]`（TRUST_DESIGN.md §6.1，里程碑 R1），IPv6 不可用时由
/// [`aa4c_proto::net::bind_tcp_dual_stack`] 自行回落纯 IPv4，行为与打通双栈之前一致。
async fn bind_with_fallback(port: u16) -> Result<TcpListener> {
    // 端口 0 = 系统分配（测试用），不做递增
    if port == 0 {
        return Ok(TcpListener::from_std(net::bind_tcp_dual_stack(0)?)?);
    }
    let mut last_err = None;
    for offset in 0..16u16 {
        let candidate = port.checked_add(offset).unwrap_or(DEFAULT_PORT);
        match net::bind_tcp_dual_stack(candidate) {
            Ok(l) => return Ok(TcpListener::from_std(l)?),
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
