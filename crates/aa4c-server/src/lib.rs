//! `aa4c-server`：自建信令 + 中继服务器（CONNECT_DESIGN.md §1.1/§3/§4，PROTOCOL.md Part C，
//! 里程碑 C2 信令面 + C3 中继面）。单进程。
//!
//! 身份复用 `aa4c-identity`（独立数据目录，与设备同构：Ed25519 密钥对 + 自签证书）。
//! 鉴权复用 mTLS——接受任意合法 Ed25519 客户端证书，从证书读出 device_id，不单独实现
//! 设计稿里的 `Challenge`/`ChallengeReply`（理由见 [`aa4c_proto::server`] 模块文档）。
//!
//! 注册表 + 中继会话表**全内存态、无持久化**：进程重启即清空，客户端靠周期续约自愈
//! （CONNECT_DESIGN.md §3.2「全内存态，无持久化」）。
//!
//! 中继面（里程碑 C3，连接阶梯第 4 档）：`RelayRequest` 换一次性 token（[`ServerMessage`]
//! 文档已说明与设计稿 `RelayData`/`RelayClose` 的收敛——匹配后直接裸字节透明转发）；
//! 若目标设备当前维持着常驻连接（`enable_remote=true` 且在线），服务器会把 `IncomingRelay`
//! 推送给它，这是 CONNECT_DESIGN.md §3.4「被叫方与其 home server 保持长连接」的落地。

#![forbid(unsafe_code)]

mod reflect;

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
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

/// 注册续约 TTL（CONNECT_DESIGN.md §12 已确认决定：60s，客户端约每 TTL/3 续约一次）。
pub const REGISTER_TTL: Duration = Duration::from_secs(60);

/// 中继会话 token 的有效期：一次性、短 TTL（CONNECT_DESIGN.md §4）。未在窗口内被两侧
/// `RelayOpen` 撮合成功即作废——足够覆盖「打洞/直连都失败后紧接着申请中继」的交互延迟，
/// 又不至于让泄露的 token 长期可用。刻意选得较短：合法的撮合只需要几个 RTT（毫秒级），
/// 这个窗口只在「对端确实不可达」时才会被真正等满——TTL 越短，这条失败路径就越快
/// 报错，而不会拖累整个连接阶梯的失败延迟（呼应 `aa4c-core::server_link` 里
/// `RELAY_OPEN_TIMEOUT` 的同一考量）。
const RELAY_TOKEN_TTL: Duration = Duration::from_secs(8);

/// 过期中继会话的清扫周期（只清「申请后从未被 `RelayOpen` 触碰」的僵尸条目——被触碰过的
/// 条目无论成败都已在触碰时移除，见 [`handle_relay_open`]）。
const RELAY_REAP_INTERVAL: Duration = Duration::from_secs(10);

struct Registration {
    endpoints: Vec<SocketAddr>,
    allow_list: HashSet<DeviceId>,
    expires_at: Instant,
}

/// 一次中继会话在服务器侧的登记状态：等待第一位到场，或已有一位在等第二位
/// （撮合用 `oneshot` 把先到者的连接交给后到者所在的任务，由后者统一完成 ack + 拼接）。
enum RelaySlot {
    Empty {
        expires_at: Instant,
    },
    FirstWaiting {
        handoff: oneshot::Sender<ServerTlsStream>,
        expires_at: Instant,
    },
}

impl RelaySlot {
    fn expires_at(&self) -> Instant {
        match self {
            RelaySlot::Empty { expires_at } | RelaySlot::FirstWaiting { expires_at, .. } => {
                *expires_at
            }
        }
    }
}

/// 服务器启动配置。
pub struct ServerConfig {
    /// 身份数据目录（`identity/device.key` 等，与设备端同一套 `aa4c-identity` 布局）。
    pub data_dir: PathBuf,
    /// 监听地址；端口 0 由系统分配（常用于测试内嵌启动）。
    pub listen_addr: SocketAddr,
}

/// 已启动的服务器句柄：持有身份与注册表，供查询状态 / 测试内嵌使用。
///
/// 后台跑三条常驻任务（TCP 接受循环、中继会话回收、反射端点），全部由 [`Server::shutdown`]
/// 统一停止。V0.7 里程碑 R4 之前这里没有优雅关闭——独立部署时进程退出即可，但桌面端把它
/// 内嵌成「可选的内置服务器」之后就不行了：用户在设置里关掉开关，端口必须真的还回去。
pub struct Server {
    identity: Arc<Identity>,
    registrations: Mutex<HashMap<DeviceId, Registration>>,
    /// 中继会话登记表：`session_token` → 撮合状态（里程碑 C3）。
    relay_sessions: Mutex<HashMap<String, RelaySlot>>,
    /// 当前维持常驻连接、可被推送 `IncomingRelay` 的设备（里程碑 C3）。客户端一侧只有
    /// `aa4c-core::server_link` 的常驻连接会发 `Register`（一次性「立即生效」的场景走
    /// `register_notify` 唤醒同一条常驻连接重新注册，不再另开一次性连接——见该模块文档：
    /// 曾经真实踩过「一次性连接的 Register 把常驻连接刚登记好的活通道顶掉」的时序问题，
    /// 现在从根上不存在第二个会调用 `Register` 的连接了），所以这里直接覆盖即可。
    pushable: Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ServerMessage>>>,
    local_addr: SocketAddr,
    /// 停机信号：接受循环与回收循环各自在下一个 select 点退出。
    shutdown: CancellationToken,
    /// 反射端点句柄（best-effort 绑定失败时为 `None`）：停机时关掉它，让那边的
    /// `accept()` 拿到 `None` 自然退出、UDP 端口释放。
    reflect_endpoint: Mutex<Option<quinn::Endpoint>>,
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

    /// 停止服务器：三条常驻任务全部退出，TCP 与 UDP 端口一并释放。
    ///
    /// 已经建立的连接不强杀，只是不再接受新连接。**幂等**，重复调用无副作用。
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(endpoint) = self
            .reflect_endpoint
            .lock()
            .expect("reflect endpoint lock")
            .take()
        {
            endpoint.close(0u32.into(), b"shutdown");
        }
    }
}

/// 启动服务器：绑定监听端口、装配身份，返回可查询状态的句柄；接受循环在后台任务里跑。
pub async fn run(config: ServerConfig) -> Result<Arc<Server>> {
    let identity = Arc::new(Identity::load_or_generate(&config.data_dir)?);
    // 接受任意合法 Ed25519 客户端证书（服务器服务多个不同设备，不固定某一个期望对端）
    let tls_config = identity.tls_server_config(None)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // 双栈监听（里程碑 R1）：默认地址 `[::]` 走 `bind_tcp_dual_stack`，它会**显式**关掉
    // `IPV6_V6ONLY`（不能赌平台默认值，见 `aa4c_proto::net`），于是同一个端口同时接受
    // IPv6 与 IPv4。只监听 IPv4 的自建服务器会把 R1 的收益吃掉一半——国内家宽普遍下发
    // 公网 IPv6，手机在 IPv6-only 的蜂窝网里根本够不着它。
    //
    // 管理员显式指定了具体地址（不是通配）时按原样绑：那是明确的意图，不该被改写。
    let listener = if config.listen_addr.is_ipv6() && config.listen_addr.ip().is_unspecified() {
        let std_listener = aa4c_proto::net::bind_tcp_dual_stack(config.listen_addr.port())
            .map_err(|e| Aa4cError::Network(format!("bind {}: {e}", config.listen_addr)))?;
        TcpListener::from_std(std_listener)
            .map_err(|e| Aa4cError::Network(format!("bind {}: {e}", config.listen_addr)))?
    } else {
        TcpListener::bind(config.listen_addr)
            .await
            .map_err(|e| Aa4cError::Network(format!("bind {}: {e}", config.listen_addr)))?
    };
    let local_addr = listener
        .local_addr()
        .map_err(|e| Aa4cError::Network(e.to_string()))?;

    let shutdown = CancellationToken::new();
    let server = Arc::new(Server {
        identity,
        registrations: Mutex::new(HashMap::new()),
        relay_sessions: Mutex::new(HashMap::new()),
        pushable: Mutex::new(HashMap::new()),
        local_addr,
        shutdown: shutdown.clone(),
        reflect_endpoint: Mutex::new(None),
    });

    let srv = server.clone();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                biased;
                () = stop.cancelled() => break,
                r = listener.accept() => r,
            };
            let (tcp, peer) = match accepted {
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

    let reaper = server.clone();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                () = tokio::time::sleep(RELAY_REAP_INTERVAL) => {}
            }
            let now = Instant::now();
            reaper
                .relay_sessions
                .lock()
                .expect("relay_sessions lock")
                .retain(|_, slot| slot.expires_at() > now);
        }
    });

    // 打洞探测端点（里程碑 C5，连接阶梯第 3 档）：best-effort，绑不上只警告——没有它
    // 只是打洞这一档失效，其余阶梯（直连/中继）不受影响，同 QUIC 对设备端的降级惯例。
    match reflect::spawn(server.identity.clone(), local_addr.port()) {
        Ok((port, endpoint)) => {
            *server
                .reflect_endpoint
                .lock()
                .expect("reflect endpoint lock") = Some(endpoint);
            tracing::info!(port, "reflect endpoint listening");
        }
        Err(e) => {
            tracing::warn!(error = %e, "reflect endpoint unavailable, NAT hole punching disabled")
        }
    }

    tracing::info!(
        addr = %local_addr,
        device_id = %server.device_id(),
        "aa4c-server listening"
    );
    Ok(server)
}

type ServerTlsStream = tokio_rustls::server::TlsStream<TcpStream>;

async fn handle_connection(server: Arc<Server>, mut stream: ServerTlsStream) -> Result<()> {
    // 还原 IPv4 映射地址（里程碑 R1，见 `aa4c_proto::net::normalize_mapped`）：这个地址
    // 会作为端点登记进注册表、再被别的设备取走用于直连，形式必须与对端自报的一致。
    let peer_addr = stream
        .get_ref()
        .0
        .peer_addr()
        .ok()
        .map(aa4c_proto::net::normalize_mapped);
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

    // 单连接内可能有多轮 Register（续约）/ Lookup/ RelayRequest；对端关闭或出错即正常退出
    // （客户端每次一次性操作都是一条独立短连接，见 aa4c-core::server_link 的 `register_once`/
    // `lookup_once`；开启远程的设备还会额外维持一条常驻连接用于收 `IncomingRelay` 推送，
    // 见下方 `push_rx` 分支与里程碑 C3）。
    let (push_tx, mut push_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let mut registered_as: Option<DeviceId> = None;
    loop {
        tokio::select! {
            biased;
            msg = read_message::<_, ServerMessage>(&mut stream) => {
                let msg = match msg {
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
                        // 登记（或续约）本连接为该设备的推送通道（里程碑 C3，见 `pushable` 字段文档）。
                        server
                            .pushable
                            .lock()
                            .expect("pushable lock")
                            .insert(cert_id.clone(), push_tx.clone());
                        registered_as = Some(cert_id.clone());
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
                    ServerMessage::RelayRequest { target } => {
                        let token = uuid::Uuid::new_v4().to_string();
                        let expires_at = Instant::now() + RELAY_TOKEN_TTL;
                        server
                            .relay_sessions
                            .lock()
                            .expect("relay_sessions lock")
                            .insert(token.clone(), RelaySlot::Empty { expires_at });
                        // best-effort 推送：target 不在线/未开启远程都静默——请求方后续
                        // RelayOpen 会在 TTL 内等不到对端而自然超时，见 ServerMessage 文档。
                        if let Some(tx) = server.pushable.lock().expect("pushable lock").get(&target) {
                            let _ = tx.send(ServerMessage::IncomingRelay {
                                session_token: token.clone(),
                                from: cert_id.clone(),
                            });
                        }
                        write_message(
                            &mut stream,
                            &ServerMessage::RelayGrant {
                                session_token: token,
                                ttl_secs: RELAY_TOKEN_TTL.as_secs(),
                            },
                        )
                        .await?;
                    }
                    ServerMessage::RelayOpen { session_token } => {
                        // 数据面：接管这条连接，不再回到本循环（无论撮合成败都直接结束本函数）。
                        return handle_relay_open(server, stream, session_token).await;
                    }
                    ServerMessage::Signal { target, candidates } => {
                        // 打洞候选转发（里程碑 C5）：纯盲转发，不回执给发起方——发起方
                        // 在自己等待 `IncomingSignal` 回信的超时里感知失败，同 RelayRequest
                        // 的防探测考量（target 不在线/未开启远程都静默，不区分原因）。
                        // 必须发在**发起方自己的常驻连接**上：回信是对端也发一条 `Signal`
                        // 给发起方，会被当作 `IncomingSignal` 推送回同一条连接
                        // （见 `aa4c-core::server_link`）。
                        if let Some(tx) = server.pushable.lock().expect("pushable lock").get(&target)
                        {
                            let _ = tx.send(ServerMessage::IncomingSignal {
                                from: cert_id.clone(),
                                candidates,
                            });
                        }
                    }
                    other => {
                        tracing::debug!(error = %unexpected(&other), "closing connection");
                        break;
                    }
                }
            }
            Some(pushed) = push_rx.recv() => {
                if write_message(&mut stream, &pushed).await.is_err() {
                    break;
                }
            }
        }
    }
    if let Some(id) = registered_as {
        let mut pushable = server.pushable.lock().expect("pushable lock");
        if pushable
            .get(&id)
            .is_some_and(|tx| tx.same_channel(&push_tx))
        {
            pushable.remove(&id);
        }
    }
    Ok(())
}

/// 中继数据面（里程碑 C3）：按 `session_token` 撮合两条 `RelayOpen` 连接，回一次
/// `RelayOpenAck` 后转入裸字节透明转发（`copy_bidirectional`），不解密、不理解 ATP。
/// Token 一次性——无论撮合成败，第一次被 `RelayOpen` 触碰即从登记表移除。
async fn handle_relay_open(
    server: Arc<Server>,
    mut stream: ServerTlsStream,
    token: String,
) -> Result<()> {
    enum Role {
        First(oneshot::Receiver<ServerTlsStream>, Instant),
        Second(oneshot::Sender<ServerTlsStream>),
    }

    let role = {
        let mut sessions = server.relay_sessions.lock().expect("relay_sessions lock");
        match sessions.remove(&token) {
            Some(RelaySlot::Empty { expires_at }) if expires_at > Instant::now() => {
                let (tx, rx) = oneshot::channel();
                sessions.insert(
                    token.clone(),
                    RelaySlot::FirstWaiting {
                        handoff: tx,
                        expires_at,
                    },
                );
                Some(Role::First(rx, expires_at))
            }
            Some(RelaySlot::FirstWaiting {
                handoff,
                expires_at,
            }) if expires_at > Instant::now() => Some(Role::Second(handoff)),
            _ => None,
        }
    };

    match role {
        None => {
            let _ = write_message(&mut stream, &ServerMessage::RelayOpenAck { ok: false }).await;
            Err(Aa4cError::Protocol("relay token unknown or expired".into()))
        }
        Some(Role::Second(handoff)) => {
            // 把自己这条连接交给先到者所在的任务；ack + 拼接都由那边统一完成。
            let _ = handoff.send(stream);
            Ok(())
        }
        Some(Role::First(rx, expires_at)) => {
            let remaining = expires_at.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, rx).await {
                Ok(Ok(mut peer_stream)) => {
                    write_message(&mut peer_stream, &ServerMessage::RelayOpenAck { ok: true })
                        .await?;
                    write_message(&mut stream, &ServerMessage::RelayOpenAck { ok: true }).await?;
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut peer_stream).await;
                    Ok(())
                }
                _ => {
                    let _ = write_message(&mut stream, &ServerMessage::RelayOpenAck { ok: false })
                        .await;
                    Err(Aa4cError::Network(
                        "relay peer never showed up before ttl".into(),
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    /// `shutdown()` 必须真的停掉后台——**监听端口释放是最容易验证的那个证据**。
    ///
    /// 独立部署时进程退出就够了，所以 R4 之前这里根本没有优雅关闭；但桌面端把它内嵌成
    /// 「可选的内置服务器」之后，用户在设置里关掉开关，端口就该真的还回去。
    #[tokio::test]
    async fn shutdown_releases_the_listening_port() {
        let dir = tempfile::tempdir().unwrap();
        let server = run(ServerConfig {
            data_dir: dir.path().to_path_buf(),
            listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        })
        .await
        .unwrap();
        let port = server.local_addr().port();

        // 探针用"连不连得上"而不是"能不能重新绑定"：`SO_REUSEADDR`（tokio 默认开）会让
        // 重新绑定在 macOS 上恒成功，那个探针测不出任何东西。
        assert!(
            tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok(),
            "运行中的服务器当然连得上"
        );

        server.shutdown();

        let mut refused = false;
        for _ in 0..50 {
            if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
            {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(refused, "shutdown 之后端口必须释放，否则接受循环还在跑");

        // 幂等：重复调用不该出错
        server.shutdown();
    }

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
            relay_sessions: Mutex::new(HashMap::new()),
            pushable: Mutex::new(HashMap::new()),
            local_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 42430)),
            shutdown: CancellationToken::new(),
            reflect_endpoint: Mutex::new(None),
        };
        let addr = server.address_with_host("example.com");
        assert!(addr.starts_with("aa4c://example.com:42430#"));
        let fp = addr.rsplit('#').next().unwrap();
        assert_eq!(fp.len(), 16);
        assert!(server.device_id().starts_with(fp));
    }

    /// B 维持常驻连接并 `Register`；A 对 B 发 `RelayRequest`：B 应在自己的常驻连接上
    /// 收到 `IncomingRelay`（里程碑 C3，CONNECT_DESIGN.md §3.4）。
    #[tokio::test]
    async fn relay_request_pushes_incoming_relay_to_registered_peer() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        // B：常驻连接，先 Register 登记推送通道
        let mut stream_b = connect_client(&server, &b).await;
        write_message(
            &mut stream_b,
            &ServerMessage::Register {
                endpoints: vec![],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec![],
            },
        )
        .await
        .unwrap();
        let _: ServerMessage = read_message(&mut stream_b).await.unwrap(); // RegisterAck

        // A：申请中继会话
        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::RelayRequest {
                target: b.device_id().clone(),
            },
        )
        .await
        .unwrap();
        let token = match timeout(
            Duration::from_secs(2),
            read_message::<_, ServerMessage>(&mut stream_a),
        )
        .await
        .unwrap()
        .unwrap()
        {
            ServerMessage::RelayGrant { session_token, .. } => session_token,
            other => panic!("unexpected: {other:?}"),
        };

        // B 在常驻连接上应收到推送
        match timeout(
            Duration::from_secs(2),
            read_message::<_, ServerMessage>(&mut stream_b),
        )
        .await
        .unwrap()
        .unwrap()
        {
            ServerMessage::IncomingRelay {
                session_token,
                from,
            } => {
                assert_eq!(session_token, token);
                assert_eq!(from, *a.device_id());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// B 维持常驻连接并 `Register`；A 在**自己的**常驻连接上发 `Signal`：B 应在自己的
    /// 常驻连接上收到 `IncomingSignal`（里程碑 C5，CONNECT_DESIGN.md §2 连接阶梯第 3 档）。
    /// 未注册/不在线的目标静默丢弃，不回执给发起方——同 `RelayRequest` 的防探测考量。
    #[tokio::test]
    async fn signal_pushes_incoming_signal_to_registered_peer() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        // 双方都维持常驻连接（A 也要注册，才能在自己这条连接上收到 B 的回信）
        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::Register {
                endpoints: vec![],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec![],
            },
        )
        .await
        .unwrap();
        let _: ServerMessage = read_message(&mut stream_a).await.unwrap(); // RegisterAck

        let mut stream_b = connect_client(&server, &b).await;
        write_message(
            &mut stream_b,
            &ServerMessage::Register {
                endpoints: vec![],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec![],
            },
        )
        .await
        .unwrap();
        let _: ServerMessage = read_message(&mut stream_b).await.unwrap(); // RegisterAck

        // A 在自己的常驻连接上发 Signal 给 B
        let a_candidates = vec!["10.0.0.5:42420".parse().unwrap()];
        write_message(
            &mut stream_a,
            &ServerMessage::Signal {
                target: b.device_id().clone(),
                candidates: a_candidates.clone(),
            },
        )
        .await
        .unwrap();

        // B 应在自己的常驻连接上收到推送
        match timeout(
            Duration::from_secs(2),
            read_message::<_, ServerMessage>(&mut stream_b),
        )
        .await
        .unwrap()
        .unwrap()
        {
            ServerMessage::IncomingSignal { from, candidates } => {
                assert_eq!(from, *a.device_id());
                assert_eq!(candidates, a_candidates);
            }
            other => panic!("unexpected: {other:?}"),
        }

        // B 回信：自己的候选经 Signal 发回，A 应在自己的常驻连接上收到 IncomingSignal
        let b_candidates = vec!["10.0.0.9:42420".parse().unwrap()];
        write_message(
            &mut stream_b,
            &ServerMessage::Signal {
                target: a.device_id().clone(),
                candidates: b_candidates.clone(),
            },
        )
        .await
        .unwrap();
        match timeout(
            Duration::from_secs(2),
            read_message::<_, ServerMessage>(&mut stream_a),
        )
        .await
        .unwrap()
        .unwrap()
        {
            ServerMessage::IncomingSignal { from, candidates } => {
                assert_eq!(from, *b.device_id());
                assert_eq!(candidates, b_candidates);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// `Signal` 目标未注册/不在线：静默丢弃，不给发起方任何回执（发起方靠自己的等待
    /// 超时感知——这里只验证「不会因为目标不存在就报错或收到什么奇怪的东西」）。
    #[tokio::test]
    async fn signal_to_unregistered_target_is_silently_dropped() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();

        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::Signal {
                target: "nope".repeat(16),
                candidates: vec![],
            },
        )
        .await
        .unwrap();
        // 没有任何回执：短暂等待确认确实没有消息到达（不是漏发，是设计如此）
        let res = timeout(
            Duration::from_millis(200),
            read_message::<_, ServerMessage>(&mut stream_a),
        )
        .await;
        assert!(
            res.is_err(),
            "should not receive anything for unknown target"
        );
    }

    /// 两条连接各自 `RelayOpen` 同一个 token：撮合后应能收发裸字节（里程碑 C3 数据面）。
    #[tokio::test]
    async fn relay_open_splices_two_matched_connections() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();

        let mut stream_a = connect_client(&server, &a).await;
        write_message(
            &mut stream_a,
            &ServerMessage::RelayRequest {
                target: b.device_id().clone(),
            },
        )
        .await
        .unwrap();
        let token = match read_message::<_, ServerMessage>(&mut stream_a)
            .await
            .unwrap()
        {
            ServerMessage::RelayGrant { session_token, .. } => session_token,
            other => panic!("unexpected: {other:?}"),
        };

        let mut open_a = connect_client(&server, &a).await;
        let mut open_b = connect_client(&server, &b).await;
        write_message(
            &mut open_a,
            &ServerMessage::RelayOpen {
                session_token: token.clone(),
            },
        )
        .await
        .unwrap();
        write_message(
            &mut open_b,
            &ServerMessage::RelayOpen {
                session_token: token,
            },
        )
        .await
        .unwrap();

        let (ack_a, ack_b) = tokio::join!(
            read_message::<_, ServerMessage>(&mut open_a),
            read_message::<_, ServerMessage>(&mut open_b),
        );
        assert!(matches!(
            ack_a.unwrap(),
            ServerMessage::RelayOpenAck { ok: true }
        ));
        assert!(matches!(
            ack_b.unwrap(),
            ServerMessage::RelayOpenAck { ok: true }
        ));

        // 撮合后这条连接转入透明转发：直接读写裸字节验证双向可达
        open_a.write_all(b"hello-from-a").await.unwrap();
        let mut buf = [0u8; 12];
        open_b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello-from-a");

        open_b.write_all(b"hello-from-b").await.unwrap();
        open_a.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello-from-b");
    }

    #[tokio::test]
    async fn relay_open_with_unknown_token_is_rejected() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let mut stream = connect_client(&server, &a).await;
        write_message(
            &mut stream,
            &ServerMessage::RelayOpen {
                session_token: "no-such-token".into(),
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut stream).await.unwrap() {
            ServerMessage::RelayOpenAck { ok } => assert!(!ok),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// 过期 token：即使从未被任何一方 `RelayOpen` 触碰过，一旦超过 TTL 也应被拒绝
    /// （里程碑 C3 验收项之一：过期 token 被拒绝）。直接往登记表塞一个已过期的 `Empty`
    /// 槽位，不必真的等 `RELAY_TOKEN_TTL` 那么久。
    #[tokio::test]
    async fn relay_open_with_expired_token_is_rejected() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();

        let token = "already-expired".to_string();
        server.relay_sessions.lock().unwrap().insert(
            token.clone(),
            RelaySlot::Empty {
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        let mut stream = connect_client(&server, &a).await;
        write_message(
            &mut stream,
            &ServerMessage::RelayOpen {
                session_token: token,
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut stream).await.unwrap() {
            ServerMessage::RelayOpenAck { ok } => assert!(!ok),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Token 一次性：撮合成功后同一 token 不能被第三方连接再次使用。
    #[tokio::test]
    async fn relay_token_is_single_use() {
        let server = start_test_server().await;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let dir_c = tempfile::tempdir().unwrap();
        let a = Identity::load_or_generate(dir_a.path()).unwrap();
        let b = Identity::load_or_generate(dir_b.path()).unwrap();
        let c = Identity::load_or_generate(dir_c.path()).unwrap();

        let mut req = connect_client(&server, &a).await;
        write_message(
            &mut req,
            &ServerMessage::RelayRequest {
                target: b.device_id().clone(),
            },
        )
        .await
        .unwrap();
        let token = match read_message::<_, ServerMessage>(&mut req).await.unwrap() {
            ServerMessage::RelayGrant { session_token, .. } => session_token,
            other => panic!("unexpected: {other:?}"),
        };

        let mut open_a = connect_client(&server, &a).await;
        let mut open_b = connect_client(&server, &b).await;
        write_message(
            &mut open_a,
            &ServerMessage::RelayOpen {
                session_token: token.clone(),
            },
        )
        .await
        .unwrap();
        write_message(
            &mut open_b,
            &ServerMessage::RelayOpen {
                session_token: token.clone(),
            },
        )
        .await
        .unwrap();
        let (ack_a, ack_b) = tokio::join!(
            read_message::<_, ServerMessage>(&mut open_a),
            read_message::<_, ServerMessage>(&mut open_b),
        );
        assert!(matches!(
            ack_a.unwrap(),
            ServerMessage::RelayOpenAck { ok: true }
        ));
        assert!(matches!(
            ack_b.unwrap(),
            ServerMessage::RelayOpenAck { ok: true }
        ));

        // C 用同一个（已被消费的）token 再来一次：应被拒绝
        let mut open_c = connect_client(&server, &c).await;
        write_message(
            &mut open_c,
            &ServerMessage::RelayOpen {
                session_token: token,
            },
        )
        .await
        .unwrap();
        match read_message::<_, ServerMessage>(&mut open_c).await.unwrap() {
            ServerMessage::RelayOpenAck { ok } => assert!(!ok),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
