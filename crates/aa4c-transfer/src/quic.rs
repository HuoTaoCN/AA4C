//! 广域网会话层：QUIC（CONNECT_DESIGN.md §5，PROTOCOL.md §10，里程碑 C1）。
//!
//! 首版单流等价迁移：每次逻辑会话建一条新 QUIC 连接 + 一条 bidi 流，在其上原样跑
//! 既有 ATP 收发循环（`client_hello`/`server_hello` 及之后的一切消息，协议层零改动）。
//! 证书固定复用 `aa4c-identity` 的 rustls 配置（同一份 mTLS 信任模型，QUIC 只是新的
//! 承载层）；ALPN 固定为 `aa4c`，防协议混淆（非强制，纯防御性加固）。
//!
//! 单任务多流（每文件独立流、并行与独立重传）留作打洞落地后的性能优化（见 §11）。

use std::net::SocketAddr;
use std::sync::Arc;

use aa4c_identity::Identity;
use aa4c_types::{Aa4cError, DeviceId, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};

use crate::TransferService;

/// QUIC 握手用的 TLS server name：无域名语义，纯 quinn API 要求非空占位；
/// 真正的身份校验靠证书指纹固定（`aa4c_identity::tls`），与 TCP 路径一致。
const SERVER_NAME: &str = "aa4c";
const ALPN: &[u8] = b"aa4c";

/// `aa4c-server` 反射端点专用 ALPN（`aa4c_server::reflect`），与设备间传输 QUIC 区分
/// （里程碑 C5，纯防御性加固，同 `ALPN` 的注释）。
const REFLECT_ALPN: &[u8] = b"aa4c-reflect";
/// 反射探测的等待上限：只是一次单向"告诉我我的地址"，不需要很久。
const REFLECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// keep-alive + 空闲超时：本应用等待用户确认接收可长达 `TransferConfig::timeout`
/// （默认 60s），比 quinn 默认的 30s 空闲超时还长——若什么都不做，慢用户会先被
/// 传输层误判「空闲」强制断连。用 keep-alive（周期性 PING）在连接真正存活时持续
/// 刷新空闲计时，同时把空闲超时收紧到远小于任何应用层等待——这样只有「keep-alive
/// 本身也送不出去」的真断连（网络分区/掉线）才会触发，不会误伤等人操作的正常等待。
fn transport_config() -> Arc<quinn::TransportConfig> {
    let mut cfg = quinn::TransportConfig::default();
    cfg.keep_alive_interval(Some(std::time::Duration::from_secs(2)));
    cfg.max_idle_timeout(Some(
        std::time::Duration::from_secs(8)
            .try_into()
            .expect("8s fits IdleTimeout"),
    ));
    Arc::new(cfg)
}

/// 一条 QUIC bidi 流拼成的双工流：`RecvStream: AsyncRead`、`SendStream: AsyncWrite`，
/// 直接喂给既有的 `client_hello`/`server_hello` 与收发循环，协议层无需感知底层是 QUIC。
///
/// **必须**随身带着 `Connection` 句柄，不能只留 `RecvStream`/`SendStream`：这两个流对象
/// 不足以独立维持连接存活，`Connection` 句柄全部丢弃后 quinn 会认为这条连接不再需要而
/// 关闭它——即使流本身还在被用。踩过这个坑：`IndexRequest` 的分流是「转交给 Core 注入的
/// 钩子后立即返回」（钩子内部自己 `tokio::spawn`，不等它跑完，见
/// `aa4c-core::dispatch::IndexServe`），如果 `run_incoming_quic` 只把流传下去、自己的
/// `connection` 局部变量随函数返回而丢弃，钩子那个后台任务还没来得及读写就会遇到
/// "connection lost"（`Offer`/`FetchRequest` 分支因为全程 `.await` 到底不受影响，只有
/// fire-and-forget 的分支会踩到）。把 `Connection` 一起装进这个类型、跟流同生共死，
/// 就不用在每个调用点都记得"顺手多存一个变量"。
pub(crate) struct QuicDuplex {
    inner: tokio::io::Join<quinn::RecvStream, quinn::SendStream>,
    _connection: quinn::Connection,
}

impl QuicDuplex {
    pub(crate) fn new(
        connection: quinn::Connection,
        recv: quinn::RecvStream,
        send: quinn::SendStream,
    ) -> Self {
        Self {
            inner: tokio::io::join(recv, send),
            _connection: connection,
        }
    }
}

impl tokio::io::AsyncRead for QuicDuplex {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for QuicDuplex {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// 建 QUIC 端点并启动接收循环：每个入站连接的第一条 bidi 流按现有分发处理
/// （`recv::run_incoming_quic`）。返回的 `Endpoint` 同时用于发起出站连接（见 [`connect`]），
/// 这是 quinn 官方推荐用法（`Endpoint::server` 文档：可同时用于收发）。
///
/// best-effort：调用方（`start_listener`）在失败时应只警告、不阻断启动——没有 QUIC
/// 只是回落到纯局域网 TCP 行为（CONNECT_DESIGN.md §2 优雅降级）。
pub(crate) fn listen(
    svc: Arc<TransferService>,
    identity: &Identity,
    port: u16,
) -> Result<quinn::Endpoint> {
    let mut rustls_server = identity.tls_server_config(None)?;
    rustls_server.alpn_protocols = vec![ALPN.to_vec()];
    let quic_server = QuicServerConfig::try_from(rustls_server)
        .map_err(|e| Aa4cError::Network(format!("quic server config: {e}")))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server));
    server_config.transport_config(transport_config());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let endpoint = quinn::Endpoint::server(server_config, addr)
        .map_err(|e| Aa4cError::Network(format!("quic bind: {e}")))?;

    let accept_endpoint = endpoint.clone();
    tokio::spawn(async move {
        loop {
            let Some(incoming) = accept_endpoint.accept().await else {
                break;
            };
            let svc = svc.clone();
            tokio::spawn(async move {
                match incoming.await {
                    Ok(connection) => {
                        if let Err(e) = crate::recv::run_incoming_quic(svc, connection).await {
                            tracing::warn!(error = %e, "quic incoming session ended with error");
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "quic handshake failed"),
                }
            });
        }
    });

    Ok(endpoint)
}

/// 向对端发起一条新 QUIC 连接并开一条 bidi 流（一次逻辑会话 = 一条新连接，与 TCP 路径
/// "每次调用新开一条 TCP 连接" 的语义一致，便于原样复用既有收发代码）。
///
/// `expect_peer` 走证书固定：指纹不符会在 QUIC 握手阶段直接失败，`Hello` 消息里的
/// 声明 id 校验（调用方仍应做）是双重保险，与 TCP 路径完全对称。
pub(crate) async fn connect(
    endpoint: &quinn::Endpoint,
    identity: &Identity,
    peer_id: &DeviceId,
    addr: SocketAddr,
) -> Result<QuicDuplex> {
    let mut rustls_client = identity.tls_client_config(Some(peer_id))?;
    rustls_client.alpn_protocols = vec![ALPN.to_vec()];
    let quic_client = QuicClientConfig::try_from(rustls_client)
        .map_err(|e| Aa4cError::Network(format!("quic client config: {e}")))?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client));
    client_config.transport_config(transport_config());

    let connecting = endpoint
        .connect_with(client_config, addr, SERVER_NAME)
        .map_err(|e| Aa4cError::Network(format!("quic connect: {e}")))?;
    let connection = connecting
        .await
        .map_err(|e| Aa4cError::Network(format!("quic handshake: {e}")))?;
    let (send, recv) = connection
        .open_bi()
        .await
        .map_err(|e| Aa4cError::Network(format!("quic open stream: {e}")))?;
    Ok(QuicDuplex::new(connection, recv, send))
}

/// 探测本机 QUIC 端点的反射地址（里程碑 C5，连接阶梯第 3 档打洞用）：连一次
/// `aa4c-server` 自带的反射端点（`aa4c_server::reflect`），读回服务器观测到的己方源
/// 地址。**必须用同一个 `endpoint`**（即真实设备间连接会用的那个 QUIC 端点/本地端口）
/// 发起——NAT 的外部映射通常按"本地端口"分配，换个端口学到的映射对后续真实打洞
/// 尝试就没有意义了。
///
/// `expected_fingerprint_prefix` 同 TCP 信令面的校验方式：TLS 层不预先固定期望对端
/// （服务器身份此前未知，只有地址里的指纹前缀），握手后手动比对。
pub(crate) async fn reflexive_addr(
    endpoint: &quinn::Endpoint,
    identity: &Identity,
    addr: SocketAddr,
    expected_fingerprint_prefix: &str,
) -> Result<SocketAddr> {
    let mut rustls_client = identity.tls_client_config(None)?;
    rustls_client.alpn_protocols = vec![REFLECT_ALPN.to_vec()];
    let quic_client = QuicClientConfig::try_from(rustls_client)
        .map_err(|e| Aa4cError::Network(format!("reflect quic client config: {e}")))?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client));

    let connecting = endpoint
        .connect_with(client_config, addr, SERVER_NAME)
        .map_err(|e| Aa4cError::Network(format!("reflect quic connect: {e}")))?;
    let connection = tokio::time::timeout(REFLECT_TIMEOUT, connecting)
        .await
        .map_err(|_| Aa4cError::Network("reflect quic handshake timeout".into()))?
        .map_err(|e| Aa4cError::Network(format!("reflect quic handshake: {e}")))?;

    let server_id = peer_device_id(&connection)?;
    if !server_id.starts_with(expected_fingerprint_prefix) {
        return Err(Aa4cError::Protocol(
            "reflect server fingerprint mismatch".into(),
        ));
    }

    let mut recv = tokio::time::timeout(REFLECT_TIMEOUT, connection.accept_uni())
        .await
        .map_err(|_| Aa4cError::Network("reflect accept uni timeout".into()))?
        .map_err(|e| Aa4cError::Network(format!("reflect accept uni: {e}")))?;
    let bytes = tokio::time::timeout(REFLECT_TIMEOUT, recv.read_to_end(64))
        .await
        .map_err(|_| Aa4cError::Network("reflect read timeout".into()))?
        .map_err(|e| Aa4cError::Network(format!("reflect read: {e}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Aa4cError::Protocol("reflect payload not utf8".into()))?;
    let observed: SocketAddr = text
        .parse()
        .map_err(|_| Aa4cError::Protocol("reflect payload not a socket addr".into()))?;
    connection.close(0u32.into(), b"done");
    Ok(observed)
}

/// 从 QUIC 连接的 mTLS 对端证书取指纹（与 TCP 路径 `device_id_from_cert` 语义相同，
/// 只是取证书的来源不同：TCP 从 `tokio_rustls` 流的 `peer_certificates()`，QUIC 从
/// `Connection::peer_identity()`——quinn 的 rustls 后端把它填成 `Vec<CertificateDer>`）。
pub(crate) fn peer_device_id(connection: &quinn::Connection) -> Result<DeviceId> {
    use tokio_rustls::rustls::pki_types::CertificateDer;

    let certs = connection
        .peer_identity()
        .and_then(|any| any.downcast::<Vec<CertificateDer<'static>>>().ok())
        .ok_or_else(|| Aa4cError::Protocol("quic peer presented no certificate".into()))?;
    let cert = certs
        .first()
        .ok_or_else(|| Aa4cError::Protocol("quic peer certificate chain is empty".into()))?;
    aa4c_identity::device_id_from_cert(cert)
}
