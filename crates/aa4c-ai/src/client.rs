//! `llama-server` 回环 HTTP 客户端（ARCHIVE_DESIGN.md §3.2）：手写极简
//! HTTP/1.1，照抄 `TransmissionClient`（`aa4c-download`）先例，不引 reqwest。
//! 三个端点：`GET /health`（就绪门）、`POST /v1/chat/completions`（OpenAI
//! 形态，支持 `stream:true` SSE）、`POST /v1/embeddings`。鉴权
//! `Authorization: Bearer <key>`。
//!
//! 与 `TransmissionClient` 的关键差异——**必须自己显式发送 `Connection:
//! close`**：`TransmissionClient` 靠 daemon 自己在响应后主动关闭连接，但
//! `llama-server`（cpp-httplib）默认走 HTTP keep-alive（AI2.0 真机抓包实测：
//! 不带这个 header 时响应带 `Keep-Alive: timeout=5, max=100`，连接不会自己
//! 关）。加上 `Connection: close` 请求头后，服务端会尊重它、在响应完成后
//! 主动关闭 socket——**即便是 `stream:true` 的分块（`Transfer-Encoding:
//! chunked`）响应也是如此**（同样真机抓包确认过），所以"读到 EOF = 响应结束"
//! 这个 D1/D2 一贯假设照样成立，不需要为 keep-alive 场景另外处理。

use aa4c_types::{Aa4cError, Result};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct LlamaClient {
    port: u16,
    auth_header: String,
}

/// [`LlamaClient::verify_auth`] 的结果。
///
/// 单列出「不是我们那个」而不是并进错误：这两种情况的处置完全不同——连不上是「再等等」，
/// 而 401 是「这个端口没戏了，换一个重来」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProbe {
    /// 200：认我们的 key，确实是我们拉起的那个进程。
    Ours,
    /// 401：端口上蹲着**别人的** llama-server（另一个实例 / 崩溃残留的孤儿 / 别的软件）。
    NotOurs,
}

impl LlamaClient {
    pub fn new(port: u16, api_key: &str) -> Self {
        Self {
            port,
            auth_header: format!("Bearer {api_key}"),
        }
    }

    /// 就绪门：`{"status":"ok"}` 才算通过，其余一律 `Unavailable`（同下载
    /// 引擎健康检查失败的既有错误语义）。
    pub async fn health(&self) -> Result<()> {
        let resp = self.request("GET", "/health", None).await?;
        if resp.status == 200 {
            Ok(())
        } else {
            Err(Aa4cError::Unavailable(format!(
                "llama-server health check returned http {}",
                resp.status
            )))
        }
    }

    /// 确认端口那头**确实是我们自己拉起的那个** `llama-server`——即它认我们这把
    /// `LLAMA_API_KEY`。
    ///
    /// 为什么 [`Self::health`] 不够：llama.cpp 的 `/health` **不受 API key 保护**（实测：
    /// 不带任何 Authorization 也返回 200；`/v1/models` 同样不受保护，`/props` 与 `/slots`
    /// 才受）。而端口是 `probe_free_port` 探来的——「绑 :0 读端口再释放」与引擎真正 bind
    /// 之间有竞态窗口，那个端口上完全可能是**别人的** llama-server（另一个 AA4C 实例、
    /// 上次崩溃残留的孤儿、或者别的软件）。只看 `/health` 的话它照样返 200，我们判定
    /// 「就绪」，然后第一次真正调用直接 401——健康检查还一路报着「正常」。
    ///
    /// 401 单独用 [`Aa4cError::Unauthorized`] 表达，好让 `spawn` 能把它与「还没起来」
    /// 区分开：前者重试同一个端口毫无意义，得换个端口重来。
    pub async fn verify_auth(&self) -> Result<AuthProbe> {
        let resp = self.request("GET", "/props", None).await?;
        match resp.status {
            200 => Ok(AuthProbe::Ours),
            401 => Ok(AuthProbe::NotOurs),
            other => Err(Aa4cError::Unavailable(format!(
                "llama-server /props returned http {other}"
            ))),
        }
    }

    pub async fn chat_completion(&self, request: Value) -> Result<Value> {
        self.post_json("/v1/chat/completions", request).await
    }

    pub async fn embeddings(&self, request: Value) -> Result<Value> {
        self.post_json("/v1/embeddings", request).await
    }

    async fn post_json(&self, path: &str, body: Value) -> Result<Value> {
        let body = serde_json::to_vec(&body).map_err(|e| {
            Aa4cError::Network(format!("failed to encode llama-server request: {e}"))
        })?;
        let resp = self.request("POST", path, Some(&body)).await?;
        if resp.status != 200 {
            return Err(Aa4cError::Network(format!(
                "llama-server {path} returned http {}: {}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            )));
        }
        serde_json::from_slice(&resp.body).map_err(|e| {
            Aa4cError::Network(format!("llama-server {path} returned invalid json: {e}"))
        })
    }

    async fn request(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<HttpResponse> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .map_err(Aa4cError::Io)?;

        let mut head = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Authorization: {}\r\n\
             Connection: close\r\n",
            self.auth_header
        );
        if let Some(b) = body {
            head.push_str("Content-Type: application/json\r\n");
            head.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        head.push_str("\r\n");

        stream
            .write_all(head.as_bytes())
            .await
            .map_err(Aa4cError::Io)?;
        if let Some(b) = body {
            stream.write_all(b).await.map_err(Aa4cError::Io)?;
        }

        let (status, headers, mut body) = read_response_head(&mut stream).await?;
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.map_err(Aa4cError::Io)?;
        body.extend_from_slice(&rest);

        if is_chunked(&headers) {
            let mut decoder = ChunkedDecoder::new();
            body = decoder.feed(&body);
        }

        Ok(HttpResponse { status, body })
    }

    /// 流式聊天补全（SSE）：后台任务边收边解 chunked 编码 + SSE 分帧、边解边
    /// 推送——不是"整段收完再切"，真正的增量流式（AI4 知识库问答的
    /// `KbAnswerDelta` 逐字事件要用得上）。逐条推送已解析的
    /// `chat.completion.chunk` JSON 对象；遇到 `data: [DONE]` 或连接关闭时
    /// channel 自然结束（sender 被 drop，接收端 `recv()` 返回 `None`）。
    pub fn chat_completion_stream(
        &self,
        request: Value,
    ) -> Result<mpsc::UnboundedReceiver<Result<Value>>> {
        let port = self.port;
        let auth_header = self.auth_header.clone();
        let body = serde_json::to_vec(&request).map_err(|e| {
            Aa4cError::Network(format!("failed to encode llama-server request: {e}"))
        })?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if let Err(e) = stream_chat_completion(port, &auth_header, &body, &tx).await {
                let _ = tx.send(Err(e));
            }
        });
        Ok(rx)
    }
}

async fn stream_chat_completion(
    port: u16,
    auth_header: &str,
    body: &[u8],
    tx: &mpsc::UnboundedSender<Result<Value>>,
) -> Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(Aa4cError::Io)?;
    let head = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Authorization: {auth_header}\r\n\
         Content-Type: application/json\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(Aa4cError::Io)?;
    stream.write_all(body).await.map_err(Aa4cError::Io)?;

    let (status, headers, leftover) = read_response_head(&mut stream).await?;
    if status != 200 {
        return Err(Aa4cError::Network(format!(
            "llama-server /v1/chat/completions (stream) returned http {status}"
        )));
    }
    let chunked = is_chunked(&headers);

    let mut decoder = ChunkedDecoder::new();
    let mut sse_buf = String::new();
    feed_sse(
        &mut sse_buf,
        if chunked {
            decoder.feed(&leftover)
        } else {
            leftover
        },
        tx,
    )?;

    let mut read_buf = [0u8; 4096];
    loop {
        if chunked && decoder.done {
            break;
        }
        let n = stream.read(&mut read_buf).await.map_err(Aa4cError::Io)?;
        if n == 0 {
            break;
        }
        let decoded = if chunked {
            decoder.feed(&read_buf[..n])
        } else {
            read_buf[..n].to_vec()
        };
        if feed_sse(&mut sse_buf, decoded, tx)? {
            return Ok(()); // 收到 [DONE]，提前结束，不必等对方关连接。
        }
    }
    Ok(())
}

/// 把新解出的原始字节接进 SSE 累积缓冲区，切出所有已完整的事件（以空行
/// `\n\n` 分隔）逐个解析并推送。返回 `true` 表示遇到了 `data: [DONE]`
/// （调用方应立即停止，不必再等连接关闭）。
fn feed_sse(
    sse_buf: &mut String,
    decoded: Vec<u8>,
    tx: &mpsc::UnboundedSender<Result<Value>>,
) -> Result<bool> {
    if decoded.is_empty() {
        return Ok(false);
    }
    sse_buf.push_str(&String::from_utf8_lossy(&decoded));
    while let Some(pos) = sse_buf.find("\n\n") {
        let event: String = sse_buf.drain(..pos + 2).collect();
        let event = event.trim_end_matches("\n\n");
        let Some(data) = event.strip_prefix("data:").map(str::trim) else {
            continue; // 空行/心跳/非 data 行，忽略。
        };
        if data == "[DONE]" {
            return Ok(true);
        }
        match serde_json::from_str::<Value>(data) {
            Ok(v) => {
                let _ = tx.send(Ok(v));
            }
            Err(e) => {
                let _ = tx.send(Err(Aa4cError::Network(format!(
                    "llama-server sse chunk is not valid json: {e}"
                ))));
            }
        }
    }
    Ok(false)
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    })
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

/// 增量读取状态行 + header，直到 `\r\n\r\n` 分隔符——不能像 `TransmissionClient`
/// 那样等 EOF 再整段切，流式响应的 body 在 header 读完后还远没结束。返回
/// `(status, headers, leftover)`：`leftover` 是这次读取过程中已经多读进来、
/// 属于 body 开头的字节，调用方必须把它们当成 body 的第一段处理，不能丢弃。
async fn read_response_head(
    stream: &mut TcpStream,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    const SEP: &[u8; 4] = b"\r\n\r\n";
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == SEP) {
            let head = std::str::from_utf8(&buf[..pos]).map_err(|_| {
                Aa4cError::Network("malformed http response: non-utf8 headers".into())
            })?;
            let mut lines = head.split("\r\n");
            let status_line = lines
                .next()
                .ok_or_else(|| Aa4cError::Network("malformed http response: empty".into()))?;
            let status: u16 = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| {
                    Aa4cError::Network(format!("malformed http status line: {status_line}"))
                })?;
            let headers = lines
                .filter_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    Some((k.trim().to_string(), v.trim().to_string()))
                })
                .collect();
            let leftover = buf[pos + SEP.len()..].to_vec();
            return Ok((status, headers, leftover));
        }
        let n = stream.read(&mut chunk).await.map_err(Aa4cError::Io)?;
        if n == 0 {
            return Err(Aa4cError::Network(
                "connection closed before http headers completed".into(),
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// 增量式 HTTP chunked 编码解码器：喂入任意大小的原始字节，吐出已解出的
/// body 字节（可能一次吐出 0 个——正在等下一批数据补全当前 chunk）。遇到
/// 终止 chunk（`0\r\n\r\n`）后 `done` 置真。不能假设一次 `read()` 恰好读到
/// 完整的 chunk 边界——AI2.0 真机抓包看到的分块大小（`205`/`f8`/`1e3`…）与
/// TCP 读缓冲区大小没有任何关系，必须做成有状态的增量解码。
struct ChunkedDecoder {
    buf: Vec<u8>,
    done: bool,
}

impl ChunkedDecoder {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            done: false,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.done {
                break;
            }
            let Some(pos) = self.buf.windows(2).position(|w| w == b"\r\n") else {
                break;
            };
            let Ok(size_str) = std::str::from_utf8(&self.buf[..pos]) else {
                break;
            };
            let Ok(size) = usize::from_str_radix(size_str.trim(), 16) else {
                break;
            };
            let chunk_start = pos + 2;
            let chunk_end = chunk_start + size;
            let needed = chunk_end + 2; // 含 chunk 数据后的尾随 \r\n
            if self.buf.len() < needed {
                break; // 这个 chunk 还没读全，等下一批数据。
            }
            if size == 0 {
                self.done = true;
                self.buf.clear();
                break;
            }
            out.extend_from_slice(&self.buf[chunk_start..chunk_end]);
            self.buf.drain(..needed);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_decoder_handles_a_single_chunk_delivered_whole() {
        let mut d = ChunkedDecoder::new();
        let out = d.feed(b"5\r\nhello\r\n0\r\n\r\n");
        assert_eq!(out, b"hello");
        assert!(d.done);
    }

    #[test]
    fn chunked_decoder_handles_multiple_chunks() {
        let mut d = ChunkedDecoder::new();
        let out = d.feed(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n");
        assert_eq!(out, b"hello world");
        assert!(d.done);
    }

    /// AI2.0 真机抓包实测确认过：分块大小与 TCP 读缓冲区边界无关，解码器
    /// 必须能在任意字节边界处被切开喂入仍然正确——这是最容易在"本机测试
    /// 恰好一次读全"时被掩盖的一类 bug。
    #[test]
    fn chunked_decoder_handles_split_across_arbitrary_byte_boundaries() {
        let full = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        for split in 1..full.len() {
            let mut d = ChunkedDecoder::new();
            let mut out = d.feed(&full[..split]);
            out.extend(d.feed(&full[split..]));
            assert_eq!(out, b"hello world", "split at {split} failed");
            assert!(d.done, "split at {split} did not finish");
        }
    }

    #[test]
    fn sse_feed_parses_one_complete_event() {
        let mut buf = String::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let done = feed_sse(
            &mut buf,
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n".to_vec(),
            &tx,
        )
        .unwrap();
        assert!(!done);
        let v = rx.try_recv().unwrap().unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"], "hi");
        assert!(buf.is_empty());
    }

    #[test]
    fn sse_feed_buffers_a_partial_event_until_completed() {
        let mut buf = String::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        feed_sse(&mut buf, b"data: {\"a\":1".to_vec(), &tx).unwrap();
        assert!(
            rx.try_recv().is_err(),
            "should not emit before the event is complete"
        );
        feed_sse(&mut buf, b"}\n\n".to_vec(), &tx).unwrap();
        let v = rx.try_recv().unwrap().unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn sse_feed_signals_done_on_done_marker() {
        let mut buf = String::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let done = feed_sse(&mut buf, b"data: [DONE]\n\n".to_vec(), &tx).unwrap();
        assert!(done);
    }

    #[test]
    fn read_head_and_body_roundtrip_over_a_real_socket() {
        // 用一个真实的本地 TCP 服务器（不 mock）验证 head/body 切分——项目
        // 一贯的"不 mock，真实进程/真实连接测试"惯例（同 transmission_rpc
        // 的既有测试写法，但这里连的是真 socket 不是纯字符串解析）。
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 1024];
                let _ = sock.read(&mut buf).await.unwrap();
                sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                    .await
                    .unwrap();
            });
            let mut client = TcpStream::connect(addr).await.unwrap();
            client.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
            let (status, headers, leftover) = read_response_head(&mut client).await.unwrap();
            assert_eq!(status, 200);
            assert!(headers
                .iter()
                .any(|(k, v)| k == "Content-Length" && v == "5"));
            assert_eq!(leftover, b"hello");
        });
    }
}
