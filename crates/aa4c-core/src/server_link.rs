//! 自建服务器客户端接入（CONNECT_DESIGN.md §3/§4，PROTOCOL.md Part C，里程碑 C2 信令 +
//! C3 中继）。
//!
//! 鉴权复用 mTLS：连接时不在 TLS 层做证书固定（服务器身份此前未知），握手后从对端
//! 证书读出 device_id，与 `server_url` 里的指纹前缀比对（CONNECT_DESIGN §3.1）；
//! 不实现设计稿里的 `Challenge`/`ChallengeReply`，理由见 `aa4c_proto::server` 模块文档。
//!
//! `register_once`/`lookup_once` 仍是一次性短连接（连接 → `SrvHello` → 一次
//! `Register`/`Lookup` → 断开），分别用于 `resolve_peer` 的远程兜底查询、以及
//! `server_link` 自身单测直接验证协议语义。[`spawn_register_loop`] 额外维持一条
//! **常驻**连接（里程碑 C3）：在同一条连接上周期性续约 `Register`，并 `select!` 着监听
//! 服务器推送的 `IncomingRelay`——这是 CONNECT_DESIGN.md §3.4「被叫方与其 home server
//! 保持长连接，信令可达」的落地，也是中继（连接阶梯第 4 档）能被对端主动联系到的前提。
//! 断连/出错即退避重连。
//!
//! **设置变更 / 解除配对等「立即生效」场景不再另开一次性连接去 `Register`**——早期实现
//! 这样做过，但会与常驻连接的自身注册竞争：一次性连接发完 `Register` 立刻断开，若它
//! 恰好在常驻连接之后抢到了 `pushable` 登记，断开时的清理会把常驻连接刚登记好的活
//! 通道顶掉，直到常驻连接的下一轮周期续约（最长 TTL/3）才能恢复——这段窗口内推送会
//! 悄悄丢失（实测踩到的真实竞态，见 `spawn_register_loop` 返回的 `Notify` 用法）。
//! 现在统一用 `notify_one()` 唤醒常驻连接自己立刻重新注册：只有一条连接会调用
//! `Register`，从根上不存在竞争。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use aa4c_identity::{device_id_from_cert, Identity};
use aa4c_proto::server::{unexpected, ServerMessage, SERVER_PROTO_VERSION};
use aa4c_proto::{read_message, write_message};
use aa4c_store::Store;
use aa4c_transfer::{RelayDialFuture, RelayDialer, SharedStream, TransferService};
use aa4c_types::{Aa4cError, DeviceId, Result, ServerAddr};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

const OP_TIMEOUT: Duration = Duration::from_secs(10);

/// 未开启远程 / 未配置服务器 / 上次失败时的重新检查间隔（用于感知设置变化，以及
/// 常驻连接断线后的重连间隔）。刻意选得很短：`enable_remote` 从关到开必须尽快让常驻
/// 连接接上（否则中继推送在这段窗口内完全收不到，见 `nudge_register` 只是一次性短连接、
/// 不能替代常驻连接本身及时建立）；自建服务器是个人自托管场景，高频探测的开销可忽略。
const IDLE_POLL: Duration = Duration::from_secs(2);

/// 等待 `RelayOpenAck` 的超时：略大于服务器侧的中继 token TTL（8s，见 `aa4c_server`），
/// 因为撮合可能要等到对端也完成它自己的 `RelayOpen` 才会回 ack。刻意选得较短：真正可达
/// 的对端几个 RTT 就能撮合上；对端确实不可达时，这就是连接阶梯第 4 档失败前的最长等待。
const RELAY_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// 连接服务器并完成 `SrvHello` 握手；校验证书指纹前缀。返回可继续读写 `ServerMessage` 的流。
async fn connect(
    identity: &Identity,
    addr: &ServerAddr,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = timeout(
        OP_TIMEOUT,
        TcpStream::connect((addr.host.as_str(), addr.port)),
    )
    .await
    .map_err(|_| Aa4cError::Network("server connect timeout".into()))??;
    // 服务器不在 devices 表里，先不在 TLS 层固定期望对端；连接后按地址里的指纹前缀比对。
    let config = identity.tls_client_config(None)?;
    let mut stream = TlsConnector::from(Arc::new(config))
        .connect(
            tokio_rustls::rustls::pki_types::ServerName::try_from("aa4c").expect("static name"),
            tcp,
        )
        .await?;

    let server_id = {
        let cert = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| Aa4cError::Protocol("server presented no certificate".into()))?;
        device_id_from_cert(cert)?
    };
    if !server_id.starts_with(&addr.fingerprint_prefix) {
        return Err(Aa4cError::Protocol("server fingerprint mismatch".into()));
    }

    write_message(
        &mut stream,
        &ServerMessage::SrvHello {
            server_proto: SERVER_PROTO_VERSION,
        },
    )
    .await?;
    match timeout(OP_TIMEOUT, read_message::<_, ServerMessage>(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("srv hello timeout".into()))??
    {
        ServerMessage::SrvHelloAck { .. } => Ok(stream),
        other => Err(unexpected(&other)),
    }
}

/// 本机候选端点（自报告）：回环地址打头（同机/回环测试必中），随后是尽力探测到的
/// 主要本地 IP（UDP connect 路由查询，不实际发包；沙箱/离线环境测不出就只剩回环这条）。
fn local_candidate_endpoints(port: u16) -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::from(([127, 0, 0, 1], port))];
    if let Some(ip) = primary_local_ip() {
        let candidate = SocketAddr::new(ip, port);
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// "我的 OS 会用哪个本地 IP 出网" 的零依赖探测：UDP `connect` 只做内核路由查表，
/// 不实际发包，离线也能返回结果（除非连本地路由表都没有默认路由）。
fn primary_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// 注册本机端点 + 当前已配对设备允许名单；成功返回服务器建议的续约 TTL（秒）。
///
/// 生产路径不再调用它（常驻连接的注册逻辑内联在 [`run_persistent_session`] 里，见模块
/// 文档「不再另开一次性连接」）——只留给下面的确定性单测直接验证 `Register`/`RegisterAck`
/// 协议语义，故整个函数仅在测试构建里编译。
#[cfg(test)]
pub(crate) async fn register_once(
    identity: &Identity,
    server_url: &str,
    listen_port: u16,
    allow_list: Vec<DeviceId>,
) -> Result<u64> {
    let addr = ServerAddr::parse(server_url)?;
    let mut stream = connect(identity, &addr).await?;
    write_message(
        &mut stream,
        &ServerMessage::Register {
            endpoints: local_candidate_endpoints(listen_port),
            proto: aa4c_types::PROTO_VERSION,
            allow_list,
        },
    )
    .await?;
    match timeout(OP_TIMEOUT, read_message::<_, ServerMessage>(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("register ack timeout".into()))??
    {
        ServerMessage::RegisterAck { ttl_secs } => Ok(ttl_secs),
        other => Err(unexpected(&other)),
    }
}

/// 查询目标设备当前端点；未注册 / 不在对方允许名单内 / 已过期都表现为空列表（不区分原因）。
pub(crate) async fn lookup_once(
    identity: &Identity,
    server_url: &str,
    device_id: &DeviceId,
) -> Result<Vec<SocketAddr>> {
    let addr = ServerAddr::parse(server_url)?;
    let mut stream = connect(identity, &addr).await?;
    write_message(
        &mut stream,
        &ServerMessage::Lookup {
            device_id: device_id.clone(),
        },
    )
    .await?;
    match timeout(OP_TIMEOUT, read_message::<_, ServerMessage>(&mut stream))
        .await
        .map_err(|_| Aa4cError::Network("lookup reply timeout".into()))??
    {
        ServerMessage::LookupReply { endpoints } => Ok(endpoints),
        other => Err(unexpected(&other)),
    }
}

/// 中继数据面拨号（里程碑 C3）：在一条**新**连接上发 `RelayOpen{token}`，等待撮合
/// （对端也完成它自己的 `RelayOpen` 后服务器才会回 ack），成功后这条连接即是中继裸管道
/// （`aa4c_transfer::TransferService::dial`/`accept_external` 会在其上再叠一层设备间 TLS）。
async fn relay_open(
    identity: &Identity,
    addr: &ServerAddr,
    session_token: String,
) -> Result<SharedStream> {
    let mut stream = connect(identity, addr).await?;
    write_message(&mut stream, &ServerMessage::RelayOpen { session_token }).await?;
    match timeout(
        RELAY_OPEN_TIMEOUT,
        read_message::<_, ServerMessage>(&mut stream),
    )
    .await
    .map_err(|_| Aa4cError::Network("relay open ack timeout".into()))??
    {
        ServerMessage::RelayOpenAck { ok: true } => Ok(Box::new(stream)),
        ServerMessage::RelayOpenAck { ok: false } => Err(Aa4cError::Network(
            "relay session rejected or expired".into(),
        )),
        other => Err(unexpected(&other)),
    }
}

/// 连接阶梯第 4 档的拨号入口（里程碑 C3）：先向（简化模型下自己配置的）服务器申请
/// 中继会话拿 token，再用它 `RelayOpen`。目标是否可达/在线都不在这里区分成败——不可达时
/// 只是在 [`relay_open`] 的超时窗口内等不到对端而失败，效果一致（见 `ServerMessage` 文档）。
async fn relay_dial(
    identity: &Identity,
    server_url: &str,
    peer_id: &DeviceId,
) -> Result<SharedStream> {
    let addr = ServerAddr::parse(server_url)?;
    let mut req_stream = connect(identity, &addr).await?;
    write_message(
        &mut req_stream,
        &ServerMessage::RelayRequest {
            target: peer_id.clone(),
        },
    )
    .await?;
    let token = match timeout(
        OP_TIMEOUT,
        read_message::<_, ServerMessage>(&mut req_stream),
    )
    .await
    .map_err(|_| Aa4cError::Network("relay grant timeout".into()))??
    {
        ServerMessage::RelayGrant { session_token, .. } => session_token,
        other => return Err(unexpected(&other)),
    };
    relay_open(identity, &addr, token).await
}

/// 把服务器推送来的一次 `IncomingRelay` 变成一条入站连接，交给传输层的统一分流
/// （`TransferService::accept_external`，里程碑 C3）。失败只记日志——申请方（对端）会在
/// 自己的 `RelayOpen` 超时窗口里感知失败，不需要这里再报告什么。
fn spawn_relay_accept(
    identity: Arc<Identity>,
    addr: ServerAddr,
    session_token: String,
    from: DeviceId,
    transfer: Arc<TransferService>,
) {
    tokio::spawn(async move {
        match relay_open(&identity, &addr, session_token).await {
            Ok(stream) => transfer.accept_external(stream),
            Err(e) => {
                tracing::debug!(error = %e, from = %from, "failed to accept incoming relay")
            }
        }
    });
}

/// 中继拨号器：注入给 `aa4c-transfer::TransferService`，实现连接阶梯第 4 档
/// （里程碑 C3）。只读当前设置，不持有任何长期状态。
pub(crate) struct RelayDialerImpl {
    store: Store,
    identity: Arc<Identity>,
    fallback_name: String,
    fallback_save_dir: String,
}

impl RelayDialerImpl {
    pub(crate) fn new(
        store: Store,
        identity: Arc<Identity>,
        fallback_name: String,
        fallback_save_dir: String,
    ) -> Self {
        Self {
            store,
            identity,
            fallback_name,
            fallback_save_dir,
        }
    }
}

impl RelayDialer for RelayDialerImpl {
    fn dial(&self, peer_id: DeviceId) -> RelayDialFuture {
        let store = self.store.clone();
        let identity = self.identity.clone();
        let fallback_name = self.fallback_name.clone();
        let fallback_save_dir = self.fallback_save_dir.clone();
        Box::pin(async move {
            let settings =
                crate::settings::load(&store, &fallback_name, &fallback_save_dir).await?;
            if !settings.enable_remote {
                return Err(Aa4cError::Network(
                    "remote not enabled, no relay available".into(),
                ));
            }
            let server_url = settings.server_url.ok_or_else(|| {
                Aa4cError::Network("no server configured, no relay available".into())
            })?;
            relay_dial(&identity, &server_url, &peer_id).await
        })
    }
}

/// 单次注册尝试：读取当前设置，未开启/未配置直接跳过；成功返回服务器建议的续约间隔。
/// 后台常驻连接循环（里程碑 C3）：未开启远程 / 未配置服务器时按 `IDLE_POLL` 轮询感知
/// 设置变化；一旦开启，建一条连接并在其上按 TTL/3 周期续约 `Register`，同时
/// `select!` 监听服务器推送的 `IncomingRelay`（CONNECT_DESIGN.md §3.2/§3.4）。
/// 连接断开/出错即退避 `IDLE_POLL` 后重连——不做指数退避，中继场景对重连及时性更敏感，
/// 固定间隔足够简单且可预期。
///
/// 返回一个 `Notify`：设置变更 / 解除配对等需要「立即生效」的操作应 `notify_one()`
/// 唤醒本循环——不管它当前是在「未开启，睡轮询间隔」还是「已连接，等下次续约」，都会
/// 立刻重新检查设置 / 重新注册，而不是傻等到 `IDLE_POLL`/续约窗口自然到期
/// （这不只是体验优化：`IncomingRelay` 推送要靠这条常驻连接活着才收得到，见
/// `RelayDialerImpl`；等轮询周期会在「刚解除配对就需要中继」这类场景里造成真实的
/// 时间窗口，中继请求会因为对端的常驻连接还没重新连上而找不到人）。
pub(crate) fn spawn_register_loop(
    store: Store,
    identity: Arc<Identity>,
    listen_port: u16,
    fallback_name: String,
    fallback_save_dir: String,
    transfer: Arc<TransferService>,
) -> Arc<tokio::sync::Notify> {
    let notify = Arc::new(tokio::sync::Notify::new());
    let notify_task = notify.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_persistent_session(
                &store,
                &identity,
                listen_port,
                &fallback_name,
                &fallback_save_dir,
                &transfer,
                &notify_task,
            )
            .await
            {
                tracing::debug!(error = %e, "persistent server session ended, will retry");
            }
            tokio::select! {
                () = tokio::time::sleep(IDLE_POLL) => {}
                () = notify_task.notified() => {}
            }
        }
    });
    notify
}

/// 一次「建连 → 周期续约 + 监听推送」的完整会话；返回即代表这条连接已经不能用了
/// （未开启远程时立即 `Ok(())` 短路，外层等 `IDLE_POLL` 或 `notify` 后重新检查设置）。
async fn run_persistent_session(
    store: &Store,
    identity: &Arc<Identity>,
    listen_port: u16,
    fallback_name: &str,
    fallback_save_dir: &str,
    transfer: &Arc<TransferService>,
    notify: &tokio::sync::Notify,
) -> Result<()> {
    let settings = crate::settings::load(store, fallback_name, fallback_save_dir).await?;
    if !settings.enable_remote {
        return Ok(());
    }
    let server_url = settings
        .server_url
        .ok_or_else(|| Aa4cError::Network("enable_remote but no server_url configured".into()))?;
    let addr = ServerAddr::parse(&server_url)?;
    let mut stream = connect(identity, &addr).await?;

    loop {
        let allow_list: Vec<DeviceId> = store
            .list_paired_devices()
            .await?
            .into_iter()
            .map(|d| d.id)
            .collect();
        write_message(
            &mut stream,
            &ServerMessage::Register {
                endpoints: local_candidate_endpoints(listen_port),
                proto: aa4c_types::PROTO_VERSION,
                allow_list,
            },
        )
        .await?;
        let ttl_secs = match timeout(OP_TIMEOUT, read_message::<_, ServerMessage>(&mut stream))
            .await
            .map_err(|_| Aa4cError::Network("register ack timeout".into()))??
        {
            ServerMessage::RegisterAck { ttl_secs } => ttl_secs,
            other => return Err(unexpected(&other)),
        };
        let renew_after = Duration::from_secs((ttl_secs / 3).max(3));
        let deadline = tokio::time::Instant::now() + renew_after;

        // 在下次续约前的窗口里，一边等超时/被 notify 唤醒一边监听服务器推送的 IncomingRelay
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                () = notify.notified() => break,
                msg = read_message::<_, ServerMessage>(&mut stream) => {
                    match msg? {
                        ServerMessage::IncomingRelay { session_token, from } => {
                            spawn_relay_accept(
                                identity.clone(),
                                addr.clone(),
                                session_token,
                                from,
                                transfer.clone(),
                            );
                        }
                        other => {
                            tracing::debug!(error = %unexpected(&other), "unexpected message on persistent server link");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! 直接对 `register_once`/`lookup_once` 打一个真实内嵌 `aa4c-server`，完全不涉及
    //! mDNS/Core/配对——`crates/aa4c-core/tests/core.rs` 里的全链路 e2e 测试跑在真机
    //! 上时 mDNS 组播确实能找到本机其它 Core 实例，无法用来干净地证明"仅靠 Lookup"，
    //! 所以这套协议本身的正确性（含允许名单随注册覆盖而自然吊销）由这里的确定性单测
    //! 兜底；服务端校验逻辑另有 `aa4c-server` 自己的单测覆盖。

    use super::*;
    use std::net::Ipv4Addr;

    async fn start_test_server() -> (Arc<aa4c_server::Server>, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let server = aa4c_server::run(aa4c_server::ServerConfig {
            data_dir: dir.path().to_path_buf(),
            listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        })
        .await
        .unwrap();
        let url = server.address_with_host("127.0.0.1");
        (server, dir, url)
    }

    #[tokio::test]
    async fn register_then_lookup_round_trips_endpoint() {
        let (_server, _dir, url) = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        let ttl = register_once(&b, &url, 42420, vec![a.device_id().clone()])
            .await
            .unwrap();
        assert_eq!(ttl, aa4c_server::REGISTER_TTL.as_secs());

        let endpoints = lookup_once(&a, &url, b.device_id()).await.unwrap();
        assert!(endpoints.contains(&SocketAddr::from((Ipv4Addr::LOCALHOST, 42420))));
    }

    #[tokio::test]
    async fn reregistering_without_a_peer_revokes_its_lookup() {
        // CONNECT_DESIGN.md §3.3「吊销自然发生」：解除配对后下一次注册的名单里没有对方，
        // 查询立刻查不到——不需要任何显式吊销协议，整体替换语义本身就是吊销机制。
        let (_server, _dir, url) = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        register_once(&b, &url, 42420, vec![a.device_id().clone()])
            .await
            .unwrap();
        assert!(!lookup_once(&a, &url, b.device_id())
            .await
            .unwrap()
            .is_empty());

        // B "解除配对"：重新注册，允许名单里不再有 A
        register_once(&b, &url, 42420, vec![]).await.unwrap();
        assert!(lookup_once(&a, &url, b.device_id())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn lookup_denied_when_not_in_allow_list() {
        let (_server, _dir, url) = start_test_server().await;
        let dir_b = tempfile::tempdir().unwrap();
        let dir_c = tempfile::tempdir().unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();
        let c = Identity::load_or_generate(dir_c.path()).unwrap();

        register_once(&b, &url, 42420, vec![]).await.unwrap(); // 名单里没有 C
        assert!(lookup_once(&c, &url, b.device_id())
            .await
            .unwrap()
            .is_empty());
    }
}
