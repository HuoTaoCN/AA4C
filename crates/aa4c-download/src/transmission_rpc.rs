//! Transmission JSON-RPC 客户端（DOWNLOAD_DESIGN.md §3.6.3）。与 `Aria2Client`
//! （JSON-RPC over WebSocket、有事件推送、token 鉴权）是完全不同的模型，独立
//! 实现，不硬套：
//! - **传输**：HTTP POST 单端点（`/transmission/rpc`），请求/响应都是 JSON。
//!   不引 reqwest——回环、单端点、纯 POST、无 TLS、无重定向，手写一个几十行的
//!   极简 HTTP/1.1 客户端就够了（同 D1 手写测试 HTTP 服务器的先例）。每次调用
//!   开一条新连接（`Connection: close`），不做连接池——回环延迟可忽略，调用
//!   频率是"数秒级轮询 + 用户操作触发"这个量级，简单更重要。
//! - **鉴权**：HTTP Basic（`settings.json` 里的随机用户名/密码）+
//!   `X-Transmission-Session-Id` header——首次请求必然收到 409（Transmission
//!   的 CSRF 防护设计如此），从 409 响应的 header 里取 session id，之后每次
//!   请求带上；客户端缓存这个 id，只有再次收到 409（id 过期）才重新取，不是
//!   每次调用都先探测一轮。
//! - **没有事件推送**：不是缺陷——D1 已经把"WS 通知触发"与"轮询兜底"收敛成
//!   同一个幂等 `reconcile()`（见 `lib.rs`），BT 侧直接以轮询为主路径即可。

use aa4c_types::{Aa4cError, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

pub struct TransmissionClient {
    port: u16,
    auth_header: String,
    session_id: Mutex<Option<String>>,
}

impl TransmissionClient {
    pub fn new(port: u16, username: &str, password: &str) -> Self {
        use base64::Engine as _;
        let creds =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        Self {
            port,
            auth_header: format!("Basic {creds}"),
            session_id: Mutex::new(None),
        }
    }

    /// 发起一次 RPC 调用。`arguments` 是 Transmission 方法自己的参数对象
    /// （不是 aria2 那种位置参数数组）。成功时返回响应的 `arguments` 字段
    /// （调用方真正关心的数据——`result` 字段只是 `"success"`/错误描述）。
    pub async fn call(&self, method: &str, arguments: Value) -> Result<Value> {
        let body = serde_json::to_vec(&json!({ "method": method, "arguments": arguments }))
            .map_err(|e| {
                Aa4cError::Network(format!("failed to encode transmission rpc request: {e}"))
            })?;

        let cached = self.session_id.lock().await.clone();
        let mut resp = self.post(&body, cached.as_deref()).await?;

        if resp.status == 409 {
            // CSRF 防护：session id 缺失或过期，从这次 409 响应里取新的，
            // 缓存下来给后续调用复用，然后原样重试这次调用一次。
            let session_id = resp
                .header("x-transmission-session-id")
                .ok_or_else(|| {
                    Aa4cError::Network(
                        "transmission rpc returned 409 without a session id header".into(),
                    )
                })?
                .to_string();
            *self.session_id.lock().await = Some(session_id.clone());
            resp = self.post(&body, Some(&session_id)).await?;
        }

        if resp.status == 401 {
            return Err(Aa4cError::Network(
                "transmission rpc rejected credentials (401)".into(),
            ));
        }
        if resp.status != 200 {
            return Err(Aa4cError::Network(format!(
                "transmission rpc unexpected http status {}",
                resp.status
            )));
        }

        let parsed: Value = serde_json::from_slice(&resp.body).map_err(|e| {
            Aa4cError::Network(format!("transmission rpc returned invalid json: {e}"))
        })?;
        match parsed["result"].as_str() {
            Some("success") => Ok(parsed["arguments"].clone()),
            Some(other) => Err(Aa4cError::Network(format!(
                "transmission rpc error: {other}"
            ))),
            None => Err(Aa4cError::Network(
                "transmission rpc response missing result field".into(),
            )),
        }
    }

    async fn post(&self, body: &[u8], session_id: Option<&str>) -> Result<HttpResponse> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .map_err(Aa4cError::Io)?;

        let mut head = format!(
            "POST /transmission/rpc HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Authorization: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n",
            self.auth_header,
            body.len()
        );
        if let Some(sid) = session_id {
            head.push_str(&format!("X-Transmission-Session-Id: {sid}\r\n"));
        }
        head.push_str("\r\n");

        stream
            .write_all(head.as_bytes())
            .await
            .map_err(Aa4cError::Io)?;
        stream.write_all(body).await.map_err(Aa4cError::Io)?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.map_err(Aa4cError::Io)?;
        parse_http_response(&raw)
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// 极简 HTTP/1.1 响应解析：状态行 + header（按第一个 `:` 切分）+ body（分隔符
/// `\r\n\r\n` 之后的全部剩余字节——`Connection: close` 让"读到 EOF 就是读完了
/// 整个响应"这个假设成立，不需要处理 `Content-Length`/`Transfer-Encoding`）。
fn parse_http_response(raw: &[u8]) -> Result<HttpResponse> {
    const SEP: &[u8; 4] = b"\r\n\r\n";
    let pos = raw.windows(4).position(|w| w == SEP).ok_or_else(|| {
        Aa4cError::Network("malformed http response: no header/body separator".into())
    })?;
    let head = std::str::from_utf8(&raw[..pos])
        .map_err(|_| Aa4cError::Network("malformed http response: non-utf8 headers".into()))?;
    let body = raw[pos + SEP.len()..].to_vec();

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Aa4cError::Network("malformed http response: empty".into()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Aa4cError::Network(format!("malformed http status line: {status_line}")))?;
    let headers = lines
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_409_response_and_extracts_session_id_case_insensitively() {
        let raw = b"HTTP/1.1 409 Conflict\r\nX-Transmission-Session-Id: abc123\r\nContent-Length: 5\r\n\r\nhello";
        let resp = parse_http_response(raw).unwrap();
        assert_eq!(resp.status, 409);
        assert_eq!(resp.header("x-transmission-session-id"), Some("abc123"));
        assert_eq!(resp.body, b"hello");
    }

    #[test]
    fn parses_a_200_response_with_empty_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        let resp = parse_http_response(raw).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body.is_empty());
    }

    #[test]
    fn rejects_malformed_response_without_header_separator() {
        let raw = b"not an http response";
        assert!(parse_http_response(raw).is_err());
    }
}
