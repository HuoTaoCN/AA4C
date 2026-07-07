//! `aa4c-server`：自建信令服务器（CONNECT_DESIGN.md §1.1/§3，PROTOCOL.md Part C，
//! 里程碑 C2）。单进程；本里程碑只做信令面（注册 + 查询），中继面（Relay）留给 C3。
//!
//! 身份复用 `aa4c-identity`（独立数据目录，与设备同构：Ed25519 密钥对 + 自签证书）。
//! 鉴权复用 mTLS——接受任意合法 Ed25519 客户端证书，从证书读出 device_id，不单独实现
//! 设计稿里的 `Challenge`/`ChallengeReply`（理由见 [`aa4c_proto::server`] 模块文档）。
//!
//! 注册表**全内存态、无持久化**：进程重启即清空，客户端靠周期续约自愈
//! （CONNECT_DESIGN.md §3.2「全内存态，无持久化」）。

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aa4c_identity::{device_id_from_cert, Identity};
use aa4c_proto::server::{unexpected, ServerMessage, SERVER_PROTO_VERSION};
use aa4c_proto::{read_message, write_message};
use aa4c_types::{Aa4cError, DeviceId, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

/// 注册续约 TTL（CONNECT_DESIGN.md §12 已确认决定：60s，客户端约每 TTL/3 续约一次）。
pub const REGISTER_TTL: Duration = Duration::from_secs(60);

struct Registration {
    endpoints: Vec<SocketAddr>,
    allow_list: HashSet<DeviceId>,
    expires_at: Instant,
}

/// 服务器启动配置。
pub struct ServerConfig {
    /// 身份数据目录（`identity/device.key` 等，与设备端同一套 `aa4c-identity` 布局）。
    pub data_dir: PathBuf,
    /// 监听地址；端口 0 由系统分配（常用于测试内嵌启动）。
    pub listen_addr: SocketAddr,
}

/// 已启动的服务器句柄：持有身份与注册表，供查询状态 / 测试内嵌使用。
/// 接受循环在后台任务里跑，随进程退出结束（本里程碑不做显式优雅关闭）。
pub struct Server {
    identity: Arc<Identity>,
    registrations: Mutex<HashMap<DeviceId, Registration>>,
    local_addr: SocketAddr,
}

impl Server {
    /// 服务器 device_id（BLAKE3(公钥) hex）；地址里的指纹取其前缀。
    pub fn device_id(&self) -> &DeviceId {
        self.identity.device_id()
    }

    /// 实际监听地址（`listen_addr` 端口为 0 时由系统分配）。
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 组装 `aa4c://host:port#fp16` 形式的地址串；`host` 由调用方提供
    /// （服务器不知道自己对外可见的域名/IP，这部分是部署方的责任）。
    pub fn address_with_host(&self, host: &str) -> String {
        let fp = &self.device_id()[..16.min(self.device_id().len())];
        format!("aa4c://{host}:{}#{fp}", self.local_addr.port())
    }
}

/// 启动服务器：绑定监听端口、装配身份，返回可查询状态的句柄；接受循环在后台任务里跑。
pub async fn run(config: ServerConfig) -> Result<Arc<Server>> {
    let identity = Arc::new(Identity::load_or_generate(&config.data_dir)?);
    // 接受任意合法 Ed25519 客户端证书（服务器服务多个不同设备，不固定某一个期望对端）
    let tls_config = identity.tls_server_config(None)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(config.listen_addr)
        .await
        .map_err(|e| Aa4cError::Network(format!("bind {}: {e}", config.listen_addr)))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| Aa4cError::Network(e.to_string()))?;

    let server = Arc::new(Server {
        identity,
        registrations: Mutex::new(HashMap::new()),
        local_addr,
    });

    let srv = server.clone();
    tokio::spawn(async move {
        loop {
            let (tcp, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "accept failed");
                    continue;
                }
            };
            let acceptor = acceptor.clone();
            let srv = srv.clone();
            tokio::spawn(async move {
                match acceptor.accept(tcp).await {
                    Ok(tls) => {
                        if let Err(e) = handle_connection(srv, tls).await {
                            tracing::debug!(%peer, error = %e, "server session ended");
                        }
                    }
                    Err(e) => tracing::debug!(%peer, error = %e, "tls accept failed"),
                }
            });
        }
    });

    tracing::info!(
        addr = %local_addr,
        device_id = %server.device_id(),
        "aa4c-server listening"
    );
    Ok(server)
}

type ServerTlsStream = tokio_rustls::server::TlsStream<TcpStream>;

async fn handle_connection(server: Arc<Server>, mut stream: ServerTlsStream) -> Result<()> {
    let peer_addr = stream.get_ref().0.peer_addr().ok();
    let cert_id = {
        let certs = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| Aa4cError::Protocol("peer presented no certificate".into()))?;
        device_id_from_cert(certs)?
    };

    match read_message::<_, ServerMessage>(&mut stream).await? {
        ServerMessage::SrvHello { .. } => {
            write_message(
                &mut stream,
                &ServerMessage::SrvHelloAck {
                    server_proto: SERVER_PROTO_VERSION,
                },
            )
            .await?;
        }
        other => return Err(unexpected(&other)),
    }

    // 单连接内可能有多轮 Register（续约）/ Lookup；对端关闭或出错即正常退出
    // （客户端每次操作都是一条独立短连接，见 aa4c-core::server_link，不会长期占用）。
    loop {
        let msg = match read_message::<_, ServerMessage>(&mut stream).await {
            Ok(m) => m,
            Err(_) => break,
        };
        match msg {
            ServerMessage::Register {
                mut endpoints,
                allow_list,
                ..
            } => {
                if let Some(addr) = peer_addr {
                    if !endpoints.contains(&addr) {
                        endpoints.push(addr); // 服务器观测到的源地址：免 STUN 的反射地址候选
                    }
                }
                server
                    .registrations
                    .lock()
                    .expect("registrations lock")
                    .insert(
                        cert_id.clone(),
                        Registration {
                            endpoints,
                            allow_list: allow_list.into_iter().collect(),
                            expires_at: Instant::now() + REGISTER_TTL,
                        },
                    );
                write_message(
                    &mut stream,
                    &ServerMessage::RegisterAck {
                        ttl_secs: REGISTER_TTL.as_secs(),
                    },
                )
                .await?;
            }
            ServerMessage::Lookup { device_id } => {
                let endpoints = {
                    let regs = server.registrations.lock().expect("registrations lock");
                    regs.get(&device_id)
                        .filter(|r| {
                            r.expires_at > Instant::now() && r.allow_list.contains(&cert_id)
                        })
                        .map(|r| r.endpoints.clone())
                        .unwrap_or_default()
                };
                write_message(&mut stream, &ServerMessage::LookupReply { endpoints }).await?;
            }
            other => {
                tracing::debug!(error = %unexpected(&other), "closing connection");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::time::timeout;
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    async fn start_test_server() -> Arc<Server> {
        let dir = tempfile::tempdir().unwrap();
        run(ServerConfig {
            data_dir: dir.path().to_path_buf(),
            listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        })
        .await
        .unwrap()
    }

    async fn connect_client(
        server: &Server,
        client_identity: &Identity,
    ) -> tokio_rustls::client::TlsStream<TcpStream> {
        let tcp = TcpStream::connect(server.local_addr()).await.unwrap();
        let config = client_identity.tls_client_config(None).unwrap();
        let mut stream = TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("aa4c").unwrap(), tcp)
            .await
            .unwrap();
        write_message(
            &mut stream,
            &ServerMessage::SrvHello {
                server_proto: SERVER_PROTO_VERSION,
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut stream).await.unwrap() {
            ServerMessage::SrvHelloAck { .. } => {}
            other => panic!("unexpected: {other:?}"),
        }
        stream
    }

    #[tokio::test]
    async fn register_then_lookup_by_allowed_peer_succeeds() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        // B 注册，允许名单里包含 A
        let mut stream_b = connect_client(&server, &b).await;
        write_message(
            &mut stream_b,
            &ServerMessage::Register {
                endpoints: vec!["10.0.0.5:42420".parse().unwrap()],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec![a.device_id().clone()],
            },
        )
        .await
        .unwrap();
        match timeout(
            Duration::from_secs(2),
            read_message::<_, ServerMessage>(&mut stream_b),
        )
        .await
        .unwrap()
        .unwrap()
        {
            ServerMessage::RegisterAck { ttl_secs } => assert_eq!(ttl_secs, REGISTER_TTL.as_secs()),
            other => panic!("unexpected: {other:?}"),
        }

        // A 查询 B：应命中（在名单内），且包含自报告端点 + 服务器观测到的回环源地址
        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::Lookup {
                device_id: b.device_id().clone(),
            },
        )
        .await
        .unwrap();
        let endpoints = match read_message::<_, ServerMessage>(&mut stream_a)
            .await
            .unwrap()
        {
            ServerMessage::LookupReply { endpoints } => endpoints,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(endpoints.contains(&"10.0.0.5:42420".parse().unwrap()));
        assert!(
            endpoints.len() >= 2,
            "should also include observed source addr: {endpoints:?}"
        );
    }

    #[tokio::test]
    async fn lookup_by_non_allowed_peer_returns_empty() {
        let server = start_test_server().await;
        let dir_b = tempfile::tempdir().unwrap();
        let dir_c = tempfile::tempdir().unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();
        let c = Identity::load_or_generate(dir_c.path()).unwrap(); // 不在 B 的允许名单里

        let mut stream_b = connect_client(&server, &b).await;
        write_message(
            &mut stream_b,
            &ServerMessage::Register {
                endpoints: vec!["10.0.0.5:42420".parse().unwrap()],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec!["someone-else".repeat(6)], // 不含 c
            },
        )
        .await
        .unwrap();
        let _: ServerMessage = read_message(&mut stream_b).await.unwrap();

        let mut stream_c = connect_client(&server, &c).await;
        write_message(
            &mut stream_c,
            &ServerMessage::Lookup {
                device_id: b.device_id().clone(),
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut stream_c)
            .await
            .unwrap()
        {
            ServerMessage::LookupReply { endpoints } => assert!(endpoints.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lookup_unknown_device_returns_empty() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::Lookup {
                device_id: "nope".repeat(16),
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut stream_a)
            .await
            .unwrap()
        {
            ServerMessage::LookupReply { endpoints } => assert!(endpoints.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn address_with_host_uses_fingerprint_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::load_or_generate(dir.path()).unwrap();
        let server = Server {
            identity: Arc::new(identity),
            registrations: Mutex::new(HashMap::new()),
            local_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 42430)),
        };
        let addr = server.address_with_host("example.com");
        assert!(addr.starts_with("aa4c://example.com:42430#"));
        let fp = addr.rsplit('#').next().unwrap();
        assert_eq!(fp.len(), 16);
        assert!(server.device_id().starts_with(fp));
    }
}
