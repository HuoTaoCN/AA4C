//! aria2 JSON-RPC over WebSocket 客户端（DOWNLOAD_DESIGN.md §3.2，决策表 v2.1）。
//!
//! 单条 WS 连接同时承载指令的请求/响应与事件通知——不为发指令再引入一个 HTTP
//! 客户端依赖。请求-响应按 JSON-RPC id 关联，键控 pending 表的写法照抄
//! `aa4c-core::server_link::SignalChannel` 的先例。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aa4c_types::{Aa4cError, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::sync::CancellationToken;

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// 一条 aria2 WebSocket 通知（`aria2.onDownload*`）。
#[derive(Debug, Clone)]
pub struct Aria2Notification {
    pub method: String,
    pub gid: String,
}

/// aria2 RPC 客户端句柄。`closed()` 在底层连接断开后 resolve 一次——供上层
/// 触发重连 + 对账（DOWNLOAD_DESIGN.md §3.4：断线重连后跑同一段对账逻辑）。
pub struct Aria2Client {
    secret: String,
    next_id: AtomicU64,
    pending: PendingMap,
    outbox: mpsc::UnboundedSender<WsMessage>,
    closed: CancellationToken,
}

impl Aria2Client {
    /// 连接本地 aria2c 的 RPC 端点并起读写后台任务。`notify_tx` 接收事件通知
    /// （`aria2.onDownloadStart` 等），连接读循环退出时会 drop 掉它——上层据此
    /// 判断"漏事件了，该跑一次 tellActive 兜底对账"。
    pub async fn connect(
        port: u16,
        secret: String,
        notify_tx: mpsc::UnboundedSender<Aria2Notification>,
    ) -> Result<Arc<Self>> {
        let url = format!("ws://127.0.0.1:{port}/jsonrpc");
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| Aa4cError::Network(format!("aria2 ws connect failed: {e}")))?;
        let (mut write, mut read) = ws.split();

        let (outbox_tx, mut outbox_rx) = mpsc::unbounded_channel::<WsMessage>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let closed = CancellationToken::new();

        tokio::spawn(async move {
            while let Some(msg) = outbox_rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let pending_read = pending.clone();
        let closed_read = closed.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                let WsMessage::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = v.get("id").and_then(id_as_u64) {
                    if let Some(tx) = pending_read.lock().await.remove(&id) {
                        let _ = tx.send(v);
                    }
                } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                    if let Some(gid) = v["params"][0]["gid"].as_str() {
                        let _ = notify_tx.send(Aria2Notification {
                            method: method.to_string(),
                            gid: gid.to_string(),
                        });
                    }
                }
            }
            // 读循环退出=连接断开：清空 pending（避免调用方永久挂起），并把
            // closed 标记为已触发——`CancellationToken` 是电平触发（不是边沿
            // 触发），断开发生在上层调用 `closed()` 之前也不会丢失通知，这正是
            // 用它而非 `Notify::notify_waiters` 的原因（后者会丢失早于等待
            // 发生的通知）。
            pending_read.lock().await.clear();
            closed_read.cancel();
        });

        Ok(Arc::new(Self {
            secret,
            next_id: AtomicU64::new(1),
            pending,
            outbox: outbox_tx,
            closed,
        }))
    }

    /// 底层连接断开后 resolve；已经断开时立即返回（电平触发）。
    pub async fn closed(&self) {
        self.closed.cancelled().await;
    }

    /// 发起一次 JSON-RPC 调用，自动注入 `token:<secret>` 首参数
    /// （aria2 官方约定的认证方式，`--rpc-user`/`--rpc-passwd` 已弃用不采用）。
    pub async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut full_params = vec![json!(format!("token:{}", self.secret))];
        full_params.extend(params);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": full_params,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if self.outbox.send(WsMessage::Text(req.to_string())).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(Aa4cError::Network("aria2 rpc connection closed".into()));
        }

        let resp = tokio::time::timeout(CALL_TIMEOUT, rx)
            .await
            .map_err(|_| Aa4cError::Network(format!("aria2 rpc call timed out: {method}")))?;
        let resp = resp.map_err(|_| Aa4cError::Network("aria2 rpc connection closed".into()))?;

        if let Some(err) = resp.get("error") {
            return Err(Aa4cError::Network(format!("aria2 rpc error: {err}")));
        }
        Ok(resp["result"].clone())
    }
}

/// JSON-RPC id 可能回传成数字（我们发出去的都是），兼容性地接受字符串数字。
fn id_as_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
