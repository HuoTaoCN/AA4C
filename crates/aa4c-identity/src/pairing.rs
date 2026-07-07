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
use aa4c_types::{Aa4cError, CoreEvent, DeviceId, DeviceInfo, Result, TrustLevel};
use rustls::pki_types::ServerName;
use tokio::io::AsyncRead;
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
    /// "用户决策"环节继续。`cert_id` 为对端证书指纹（与声明公钥比对）。
    pub fn handle_dispatched(
        &self,
        stream: IncomingStream,
        cert_id: DeviceId,
        peer_device: DeviceInfo,
        peer_key: [u8; 32],
    ) -> Result<String> {
        let (session_id, decisions) = self.new_session();
        let ctx = self.session_ctx(&session_id, decisions);
        tokio::spawn(async move {
            let peer_addr = stream.get_ref().0.peer_addr().ok();
            let result =
                responder_after_request(&ctx, stream, &cert_id, peer_device, peer_key, peer_addr)
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
    async fn persist_peer(
        &self,
        device: &DeviceInfo,
        public_key: [u8; 32],
        addr: Option<SocketAddr>,
    ) -> Result<()> {
        let now = now_ms();
        // 配对默认「朋友」；若该设备此前已被标记为「完全信任」，重新配对时保留，不降级。
        // server_hint（对端 home server）同理保留旧值——配对协议本身暂不交换这个字段
        // （里程碑 C2 只落地了 schema/查询，线路层交换留待后续里程碑，见 HANDOFF.md）。
        let existing = self.store.get_device(&device.id).await?;
        let trust_level = existing
            .as_ref()
            .map(|r| r.trust_level)
            .unwrap_or(TrustLevel::Friend);
        let server_hint = existing.and_then(|r| r.server_hint);
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

    let (hello_id, _proto) = client_hello(&mut stream, ctx.identity.device_id()).await?;
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

    ctx.persist_peer(&peer_device, peer_key, Some(addr)).await?;
    Ok(peer_device.id)
}

/// 接收方会话（PROTOCOL.md §6 右列）：读取 Hello + `PairRequest`，再进入
/// 用户决策环节。成功返回对端 DeviceId。
async fn responder_session(ctx: &SessionCtx, mut stream: IncomingStream) -> Result<DeviceId> {
    let peer_addr = stream.get_ref().0.peer_addr().ok();
    let cert_id = peer_cert_id(stream.get_ref().1)?;

    let (hello_id, _proto) = server_hello(&mut stream, ctx.identity.device_id()).await?;
    if hello_id != cert_id {
        return Err(Aa4cError::Protocol("hello id != certificate id".into()));
    }

    let (peer_device, peer_key) = match ctx.recv(&mut stream).await? {
        Message::PairRequest { device, public_key } => (device, public_key),
        other => return Err(unexpected(&other)),
    };
    responder_after_request(ctx, stream, &cert_id, peer_device, peer_key, peer_addr).await
}

/// 接收方会话的用户决策段（PROTOCOL.md §6 右列后半）：声明公钥校验、双向
/// 确认、写库。Hello 与首条 `PairRequest` 已由调用方读出。
async fn responder_after_request(
    ctx: &SessionCtx,
    mut stream: IncomingStream,
    cert_id: &DeviceId,
    peer_device: DeviceInfo,
    peer_key: [u8; 32],
    peer_addr: Option<SocketAddr>,
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

    ctx.persist_peer(&peer_device, peer_key, peer_addr).await?;
    Ok(peer_device.id)
}

fn reject(reason: &str) -> Message {
    Message::PairReject {
        reason: reason.into(),
    }
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
