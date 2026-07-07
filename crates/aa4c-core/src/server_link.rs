//! 自建服务器客户端接入（CONNECT_DESIGN.md §3，PROTOCOL.md Part C，里程碑 C2）。
//!
//! 鉴权复用 mTLS：连接时不在 TLS 层做证书固定（服务器身份此前未知），握手后从对端
//! 证书读出 device_id，与 `server_url` 里的指纹前缀比对（CONNECT_DESIGN §3.1）；
//! 不实现设计稿里的 `Challenge`/`ChallengeReply`，理由见 `aa4c_proto::server` 模块文档。
//!
//! 每次操作都是一条独立短连接（连接 → `SrvHello` → 一次 `Register` 或 `Lookup` → 断开），
//! 不维护常驻信令连接——简单，且不需要处理"并发访问同一条流"的问题。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use aa4c_identity::{device_id_from_cert, Identity};
use aa4c_proto::server::{unexpected, ServerMessage, SERVER_PROTO_VERSION};
use aa4c_proto::{read_message, write_message};
use aa4c_store::Store;
use aa4c_types::{Aa4cError, DeviceId, Result, ServerAddr};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

const OP_TIMEOUT: Duration = Duration::from_secs(10);

/// 未开启远程 / 未配置服务器 / 上次失败时的常规轮询间隔（用于感知设置变化）。
const IDLE_POLL: Duration = Duration::from_secs(20);

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

/// 单次注册尝试：读取当前设置，未开启/未配置直接跳过；成功返回服务器建议的续约间隔。
async fn register_tick(
    store: &Store,
    identity: &Identity,
    listen_port: u16,
    fallback_name: &str,
    fallback_save_dir: &str,
) -> Option<u64> {
    let settings = crate::settings::load(store, fallback_name, fallback_save_dir)
        .await
        .ok()?;
    if !settings.enable_remote {
        return None;
    }
    let server_url = settings.server_url?;
    let allow_list: Vec<DeviceId> = store
        .list_paired_devices()
        .await
        .ok()?
        .into_iter()
        .map(|d| d.id)
        .collect();
    match register_once(identity, &server_url, listen_port, allow_list).await {
        Ok(ttl) => Some(ttl),
        Err(e) => {
            tracing::debug!(error = %e, "server register failed");
            None
        }
    }
}

/// 立即触发一次注册（不阻塞调用方；失败只记日志）。设置变更 / 解除配对等让允许名单或
/// 服务器地址变化的操作应调用此函数，避免等到下一次周期轮询（CONNECT_DESIGN §3.2）。
pub(crate) fn nudge_register(
    store: Store,
    identity: Arc<Identity>,
    listen_port: u16,
    fallback_name: String,
    fallback_save_dir: String,
) {
    tokio::spawn(async move {
        let _ = register_tick(
            &store,
            &identity,
            listen_port,
            &fallback_name,
            &fallback_save_dir,
        )
        .await;
    });
}

/// 后台注册续约循环：开启远程时按 TTL/3 续约，未开启/失败时用较长的常规轮询间隔
/// 感知设置变化（CONNECT_DESIGN §3.2「周期性续约」）。
pub(crate) fn spawn_register_loop(
    store: Store,
    identity: Arc<Identity>,
    listen_port: u16,
    fallback_name: String,
    fallback_save_dir: String,
) {
    tokio::spawn(async move {
        loop {
            let ttl = register_tick(
                &store,
                &identity,
                listen_port,
                &fallback_name,
                &fallback_save_dir,
            )
            .await;
            let wait = match ttl {
                Some(secs) => Duration::from_secs((secs / 3).max(3)),
                None => IDLE_POLL,
            };
            tokio::time::sleep(wait).await;
        }
    });
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
