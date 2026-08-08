//! 配对协议（PROTOCOL.md §6）。
//!
//! 状态机：Requested → PinShown → BothConfirmed → Done / Failed。
//!
//! - 配对期 TLS 不固定指纹（首次见面，`expect_peer = None`），改为校验
//!   "消息中声明的公钥/设备 ID == TLS 证书指纹"，信任由双向 PIN 人工确认建立
//! - 任一等待超过 `timeout`（默认 60 秒）→ 会话失败
//! - 成功后双方写入 devices 表（trusted = 1）
//!
//! 用户决策通过 [`PairingManager::confirm`] 注入：
//! 接收方需要两次确认（接受请求、PIN 一致），发起方一次（PIN 一致）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa4c_proto::{client_hello, read_message, server_hello, unexpected, write_message, Message};
use aa4c_store::{DeviceRecord, Store};
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, DeviceInfo, Result, TrustLevel, SERVER_HINT_PROTO_VERSION,
};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::{derive_pin, device_id_from_cert, device_id_from_public_key, Identity};

/// 事件发送端（与 aa4c-core 的事件总线同型）。
pub type EventSender = broadcast::Sender<CoreEvent>;

/// 默认会话超时（PROTOCOL.md §6）。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// 接收方传入的已完成握手的 TLS 服务端流。
pub type IncomingStream = tokio_rustls::server::TlsStream<TcpStream>;

pub struct PairingManager {
    identity: Arc<Identity>,
    self_device: DeviceInfo,
    store: Store,
    events: EventSender,
    timeout: Duration,
    /// session_id → 用户决策通道
    sessions: Arc<Mutex<HashMap<String, mpsc::Sender<bool>>>>,
}

impl PairingManager {
    pub fn new(
        identity: Arc<Identity>,
        self_device: DeviceInfo,
        store: Store,
        events: EventSender,
    ) -> Self {
        Self {
            identity,
            self_device,
            store,
            events,
            timeout: DEFAULT_TIMEOUT,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 覆盖会话超时（测试用）。
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 发起配对，立即返回 session_id；进展通过事件推送。
    pub async fn start_pairing(&self, peer: &DeviceInfo) -> Result<String> {
        let addr = peer
            .addr
            .ok_or_else(|| Aa4cError::DeviceNotFound(peer.id.clone()))?;
        let (session_id, decisions) = self.new_session();
        let ctx = self.session_ctx(&session_id, decisions);
        let peer_id = peer.id.clone();
        tokio::spawn(async move {
            let result = initiator_session(&ctx, addr).await;
            ctx.finish(result, Some(peer_id));
        });
        Ok(session_id)
    }

    /// 接收方：处理一条已完成 TLS 握手的入站配对连接，返回 session_id。
    ///
    /// 自行读取 Hello 与 `PairRequest`。配对专用监听器（测试）走此路径。
    pub fn handle_incoming(&self, stream: IncomingStream) -> Result<String> {
        let (session_id, decisions) = self.new_session();
        let ctx = self.session_ctx(&session_id, decisions);
        tokio::spawn(async move {
            let result = responder_session(&ctx, stream).await;
            ctx.finish(result, None);
        });
        Ok(session_id)
    }

    /// 接收方：处理由统一传输监听器分流过来的配对连接（M6 接线）。
    ///
    /// 监听器已完成 TLS 握手、Hello 校验并读出首条 `PairRequest`，此处从
    /// "用户决策"环节继续。`cert_id` 为对端证书指纹（与声明公钥比对）；`proto` 为
    /// 监听器 Hello 握手已协商出的版本（`PairServerHint` 交换的 gate 判断用）。
    pub fn handle_dispatched(
        &self,
        stream: IncomingStream,
        cert_id: DeviceId,
        peer_device: DeviceInfo,
        peer_key: [u8; 32],
        proto: u16,
    ) -> Result<String> {
        let (session_id, decisions) = self.new_session();
        let ctx = self.session_ctx(&session_id, decisions);
        tokio::spawn(async move {
            // 双栈监听之后，普通 IPv4 入站会以 `::ffff:a.b.c.d` 的映射形式出现；
            // 这个值会一路写进 `devices.last_addr`，不还原就会让同一台设备在打通
            // 双栈前后存出两种写法（里程碑 R1，见 `aa4c_proto::net::normalize_mapped`）。
            let peer_addr = stream
                .get_ref()
                .0
                .peer_addr()
                .ok()
                .map(aa4c_proto::net::normalize_mapped);
            let result = responder_after_request(
                &ctx,
                stream,
                &cert_id,
                peer_device,
                peer_key,
                peer_addr,
                proto,
            )
            .await;
            ctx.finish(result, None);
        });
        Ok(session_id)
    }

    /// 本端用户确认 / 拒绝（PIN 核对或接受请求）。
    pub async fn confirm(&self, session_id: &str, accept: bool) -> Result<()> {
        let tx = self
            .sessions
            .lock()
            .expect("sessions lock")
            .get(session_id)
            .cloned()
            .ok_or_else(|| Aa4cError::Protocol(format!("unknown session: {session_id}")))?;
        tx.send(accept)
            .await
            .map_err(|_| Aa4cError::Protocol("pairing session already ended".into()))
    }

    fn new_session(&self) -> (String, mpsc::Receiver<bool>) {
        let session_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel(2);
        self.sessions
            .lock()
            .expect("sessions lock")
            .insert(session_id.clone(), tx);
        (session_id, rx)
    }

    fn session_ctx(&self, session_id: &str, decisions: mpsc::Receiver<bool>) -> SessionCtx {
        SessionCtx {
            session_id: session_id.to_string(),
            identity: self.identity.clone(),
            self_device: self.self_device.clone(),
            store: self.store.clone(),
            events: self.events.clone(),
            timeout: self.timeout,
            sessions: self.sessions.clone(),
            decisions: tokio::sync::Mutex::new(decisions),
        }
    }
}

/// 单个配对会话的上下文（发起方与接收方共用）。
struct SessionCtx {
    session_id: String,
    identity: Arc<Identity>,
    self_device: DeviceInfo,
    store: Store,
    events: EventSender,
    timeout: Duration,
    sessions: Arc<Mutex<HashMap<String, mpsc::Sender<bool>>>>,
    decisions: tokio::sync::Mutex<mpsc::Receiver<bool>>,
}

impl SessionCtx {
    fn emit(&self, event: CoreEvent) {
        let _ = self.events.send(event);
    }

    /// 等待本端用户决策（confirm 注入），超时按会话失败处理。
    async fn wait_decision(&self) -> Result<bool> {
        let mut rx = self.decisions.lock().await;
        timeout(self.timeout, rx.recv())
            .await
            .map_err(|_| Aa4cError::Network("pairing timeout: waiting for user".into()))?
            .ok_or(Aa4cError::Cancelled)
    }

    /// 带超时读取下一条消息。
    async fn recv<S: AsyncRead + Unpin>(&self, stream: &mut S) -> Result<Message> {
        timeout(self.timeout, read_message(stream))
            .await
            .map_err(|_| Aa4cError::Network("pairing timeout: waiting for peer".into()))?
    }

    /// 会话收尾：清理注册表、发布结果事件。
    fn finish(&self, result: Result<DeviceId>, intended_peer: Option<DeviceId>) {
        self.sessions
            .lock()
            .expect("sessions lock")
            .remove(&self.session_id);
        match result {
            Ok(peer) => {
                tracing::info!(session = %self.session_id, peer = %peer, "pairing succeeded");
                self.emit(CoreEvent::PairingResult {
                    session_id: self.session_id.clone(),
                    peer,
                    success: true,
                });
            }
            Err(e) => {
                tracing::warn!(session = %self.session_id, error = %e, "pairing failed");
                self.emit(CoreEvent::PairingResult {
                    session_id: self.session_id.clone(),
                    peer: intended_peer.unwrap_or_default(),
                    success: false,
                });
            }
        }
    }

    fn self_public_key(&self) -> Result<[u8; 32]> {
        self.identity
            .public_key()
            .try_into()
            .map_err(|_| Aa4cError::Protocol("self public key is not 32 bytes".into()))
    }

    /// 配对成功：写入对端设备（trusted = 1）。
    ///
    /// `fresh_hint`：本次会话协商到的 `server_hint`（见 `PairServerHint` 交换，
    /// PROTOCOL.md §17）。`None` = 协商未发生（对端 proto 太旧，不认识这条消息）——
    /// 沿用已存的旧值；`Some(inner)` = 协商发生了，直接覆盖成 `inner`（`inner` 本身
    /// 也可能是 `None`，代表对端明确没配置远程服务器，同样要如实覆盖，不能因为
    /// "协商到了但值是 None"就误当成"没协商"）。
    async fn persist_peer(
        &self,
        device: &DeviceInfo,
        public_key: [u8; 32],
        addr: Option<SocketAddr>,
        fresh_hint: Option<Option<String>>,
    ) -> Result<()> {
        let now = now_ms();
        // 配对默认「朋友」；若该设备此前已被标记为「完全信任」，重新配对时保留，不降级。
        let existing = self.store.get_device(&device.id).await?;
        let trust_level = existing
            .as_ref()
            .map(|r| r.trust_level)
            .unwrap_or(TrustLevel::Friend);
        let server_hint = match fresh_hint {
            Some(inner) => inner,
            None => existing.and_then(|r| r.server_hint),
        };
        self.store
            .upsert_device(&DeviceRecord {
                id: device.id.clone(),
                name: device.name.clone(),
                platform: device.platform,
                public_key: public_key.to_vec(),
                trusted: true,
                trust_level,
                paired_at: Some(now),
                last_seen_at: Some(now),
                last_addr: addr.map(|a| a.to_string()),
                server_hint,
                created_at: 0, // 由 Store 维护
                updated_at: 0,
            })
            .await
    }
}

/// PROTOCOL.md §17：`PairConfirm`/`PairConfirm` 互相确认之后、写库之前，proto ≥
/// `SERVER_HINT_PROTO_VERSION` 时双方确定性交换 `PairServerHint`。返回本次协商到的
/// 对端 hint（`Some(_)`，供 `persist_peer` 覆盖）；proto 不够时两端都不发送、返回
/// `None`（`persist_peer` 沿用旧值，行为与旧版完全一致）。
async fn exchange_server_hint<S: AsyncRead + AsyncWrite + Unpin>(
    ctx: &SessionCtx,
    stream: &mut S,
    proto: u16,
) -> Result<Option<Option<String>>> {
    if proto < SERVER_HINT_PROTO_VERSION {
        return Ok(None);
    }
    let my_hint = my_server_hint(&ctx.store).await;
    write_message(
        stream,
        &Message::PairServerHint {
            server_hint: my_hint,
        },
    )
    .await?;
    match ctx.recv(stream).await? {
        Message::PairServerHint { server_hint } => Ok(Some(server_hint)),
        other => Err(unexpected(&other)),
    }
}

/// 校验对端消息声明的身份与 TLS 证书一致（PROTOCOL.md §6 步骤 3 前置）。
fn verify_claimed_key(
    cert_id: &DeviceId,
    device: &DeviceInfo,
    public_key: &[u8; 32],
) -> Result<()> {
    let key_id = device_id_from_public_key(public_key);
    if &key_id != cert_id || device.id != key_id {
        return Err(Aa4cError::Protocol(
            "claimed public key does not match TLS certificate".into(),
        ));
    }
    Ok(())
}

/// 发起方会话（PROTOCOL.md §6 左列）。成功返回对端 DeviceId。
async fn initiator_session(ctx: &SessionCtx, addr: SocketAddr) -> Result<DeviceId> {
    let tcp = timeout(ctx.timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| Aa4cError::Network("pairing timeout: connect".into()))??;
    let config = ctx.identity.tls_client_config(None)?;
    let mut stream = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("aa4c").expect("static name"), tcp)
        .await?;
    let cert_id = peer_cert_id(stream.get_ref().1)?;

    let (hello_id, proto) = client_hello(&mut stream, ctx.identity.device_id()).await?;
    if hello_id != cert_id {
        return Err(Aa4cError::Protocol("hello id != certificate id".into()));
    }

    write_message(
        &mut stream,
        &Message::PairRequest {
            device: ctx.self_device.clone(),
            public_key: ctx.self_public_key()?,
        },
    )
    .await?;

    let (peer_device, peer_key) = match ctx.recv(&mut stream).await? {
        Message::PairAccept { device, public_key } => (device, public_key),
        Message::PairReject { .. } => return Err(Aa4cError::PairingRejected),
        other => return Err(unexpected(&other)),
    };
    verify_claimed_key(&cert_id, &peer_device, &peer_key)?;

    // 双方独立计算并展示 PIN
    let pin = derive_pin(ctx.identity.public_key(), &peer_key);
    ctx.emit(CoreEvent::PairingPin {
        session_id: ctx.session_id.clone(),
        pin,
    });

    if !ctx.wait_decision().await? {
        let _ = write_message(&mut stream, &reject("user rejected pin")).await;
        return Err(Aa4cError::PinMismatch);
    }
    write_message(&mut stream, &Message::PairConfirm).await?;

    match ctx.recv(&mut stream).await? {
        Message::PairConfirm => {}
        Message::PairReject { .. } => return Err(Aa4cError::PairingRejected),
        other => return Err(unexpected(&other)),
    }

    let fresh_hint = exchange_server_hint(ctx, &mut stream, proto).await?;
    ctx.persist_peer(&peer_device, peer_key, Some(addr), fresh_hint)
        .await?;
    Ok(peer_device.id)
}

/// 接收方会话（PROTOCOL.md §6 右列）：读取 Hello + `PairRequest`，再进入
/// 用户决策环节。成功返回对端 DeviceId。
async fn responder_session(ctx: &SessionCtx, mut stream: IncomingStream) -> Result<DeviceId> {
    // 同上：还原 IPv4 映射地址，避免 `devices.last_addr` 出现 `::ffff:` 前缀（里程碑 R1）。
    let peer_addr = stream
        .get_ref()
        .0
        .peer_addr()
        .ok()
        .map(aa4c_proto::net::normalize_mapped);
    let cert_id = peer_cert_id(stream.get_ref().1)?;

    let (hello_id, proto) = server_hello(&mut stream, ctx.identity.device_id()).await?;
    if hello_id != cert_id {
        return Err(Aa4cError::Protocol("hello id != certificate id".into()));
    }

    let (peer_device, peer_key) = match ctx.recv(&mut stream).await? {
        Message::PairRequest { device, public_key } => (device, public_key),
        other => return Err(unexpected(&other)),
    };
    responder_after_request(
        ctx,
        stream,
        &cert_id,
        peer_device,
        peer_key,
        peer_addr,
        proto,
    )
    .await
}

/// 接收方会话的用户决策段（PROTOCOL.md §6 右列后半）：声明公钥校验、双向
/// 确认、写库。Hello 与首条 `PairRequest` 已由调用方读出；`proto` 为 Hello 已协商出的
/// 版本（`PairServerHint` 交换的 gate 判断用）。
#[allow(clippy::too_many_arguments)]
async fn responder_after_request(
    ctx: &SessionCtx,
    mut stream: IncomingStream,
    cert_id: &DeviceId,
    peer_device: DeviceInfo,
    peer_key: [u8; 32],
    peer_addr: Option<SocketAddr>,
    proto: u16,
) -> Result<DeviceId> {
    verify_claimed_key(cert_id, &peer_device, &peer_key)?;

    // 第一次用户决策：是否接受配对请求
    ctx.emit(CoreEvent::PairingRequest {
        session_id: ctx.session_id.clone(),
        peer: peer_device.clone(),
    });
    if !ctx.wait_decision().await? {
        let _ = write_message(&mut stream, &reject("user rejected request")).await;
        return Err(Aa4cError::PairingRejected);
    }

    write_message(
        &mut stream,
        &Message::PairAccept {
            device: ctx.self_device.clone(),
            public_key: ctx.self_public_key()?,
        },
    )
    .await?;

    let pin = derive_pin(ctx.identity.public_key(), &peer_key);
    ctx.emit(CoreEvent::PairingPin {
        session_id: ctx.session_id.clone(),
        pin,
    });

    // 第二次用户决策：PIN 是否一致
    if !ctx.wait_decision().await? {
        let _ = write_message(&mut stream, &reject("user rejected pin")).await;
        return Err(Aa4cError::PinMismatch);
    }
    write_message(&mut stream, &Message::PairConfirm).await?;

    match ctx.recv(&mut stream).await? {
        Message::PairConfirm => {}
        Message::PairReject { .. } => return Err(Aa4cError::PairingRejected),
        other => return Err(unexpected(&other)),
    }

    let fresh_hint = exchange_server_hint(ctx, &mut stream, proto).await?;
    ctx.persist_peer(&peer_device, peer_key, peer_addr, fresh_hint)
        .await?;
    Ok(peer_device.id)
}

fn reject(reason: &str) -> Message {
    Message::PairReject {
        reason: reason.into(),
    }
}

/// 读自己当前配置的 home server 地址，供 `PairServerHint` 交换时声明给对端
/// （PROTOCOL.md §17）。直接读 `settings` 表的两个 KV（不经 `aa4c-core::settings`
/// 整个结构体——`aa4c-identity` 不依赖 `aa4c-core`，依赖方向是反过来的），值是
/// `serde_json` 编码过的（同 `aa4c-core::settings` 的 `get_json`/`set_json` 惯例）；
/// key 名字必须与 `aa4c-core::settings::{KEY_SERVER_URL, KEY_ENABLE_REMOTE}` 的字面值
/// （`"server_url"`/`"enable_remote"`）保持一致。未配置 / 未开启远程 / 解析失败都当
/// `None`，与 `settings::load` 的默认语义一致。
async fn my_server_hint(store: &Store) -> Option<String> {
    let enable_remote: bool = store
        .get_setting("enable_remote")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(false);
    if !enable_remote {
        return None;
    }
    store
        .get_setting("server_url")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Option<String>>(&raw).ok())
        .flatten()
}

/// 从 rustls 连接中取对端证书指纹。
fn peer_cert_id(conn: &impl PeerCerts) -> Result<DeviceId> {
    let cert = conn
        .certs()
        .and_then(|c| c.first())
        .ok_or_else(|| Aa4cError::Protocol("peer presented no certificate".into()))?;
    device_id_from_cert(cert)
}

/// 统一客户端/服务端 rustls 连接的证书读取。
trait PeerCerts {
    fn certs(&self) -> Option<&[rustls::pki_types::CertificateDer<'static>]>;
}

impl PeerCerts for rustls::ClientConnection {
    fn certs(&self) -> Option<&[rustls::pki_types::CertificateDer<'static>]> {
        self.peer_certificates()
    }
}

impl PeerCerts for rustls::ServerConnection {
    fn certs(&self) -> Option<&[rustls::pki_types::CertificateDer<'static>]> {
        self.peer_certificates()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
