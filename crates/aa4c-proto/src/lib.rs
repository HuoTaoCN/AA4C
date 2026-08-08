//! AA4C 线路协议（ATP v1）：消息定义与帧编解码。
//!
//! 规范见 PROTOCOL.md §3–§4。帧格式：`[4 字节大端长度][bincode(Message)]`，
//! 帧长上限 16 MiB（超限视为协议攻击，立即报错由上层断开）。
//! `Chunk` 帧之后紧跟 `len` 字节原始文件数据（不参与 bincode，避免拷贝）。

#![forbid(unsafe_code)]

pub mod net;
pub mod server;

use aa4c_types::{Aa4cError, DeviceId, DeviceInfo, Result, TaskId, MAX_FRAME_LEN, PROTO_VERSION};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 传输文件清单条目（PROTOCOL.md §4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    /// 相对路径，'/' 分隔；接收端负责净化（拒绝 ".."、绝对路径）。
    pub rel_path: String,
    pub size: u64,
}

/// 索引交换条目（SYNC_DESIGN.md §3.3，里程碑 3）。
///
/// `rel_path` 是发送方限定好的展示路径（顶层段为来源分组，如「收到的」或共享文件夹名），
/// 接收方原样存入 `remote_index`，与本机统一视图同命名空间。只含元数据，不含内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexItem {
    pub rel_path: String,
    pub size: u64,
    pub hash: Option<String>,
}

/// ATP v1 全部消息（PROTOCOL.md §4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    // —— 握手 ——
    Hello {
        proto: u16,
        device_id: DeviceId,
    },
    HelloAck {
        proto: u16,
        device_id: DeviceId,
    },

    // —— 配对 ——
    PairRequest {
        device: DeviceInfo,
        public_key: [u8; 32],
    },
    PairAccept {
        device: DeviceInfo,
        public_key: [u8; 32],
    },
    PairConfirm,
    PairReject {
        reason: String,
    },

    // —— 传输 ——
    Offer {
        task_id: TaskId,
        files: Vec<FileMeta>,
    },
    OfferAnswer {
        task_id: TaskId,
        accept: bool,
    },
    /// 帧体之后紧跟 `len` 字节原始数据。
    Chunk {
        file_index: u32,
        offset: u64,
        len: u32,
    },
    FileDone {
        file_index: u32,
        hash: String,
    },
    FileAck {
        file_index: u32,
        ok: bool,
    },
    TaskDone {
        task_id: TaskId,
    },
    Cancel {
        task_id: TaskId,
        reason: String,
    },

    // —— 索引交换（SYNC_DESIGN.md §3.3 / 里程碑 3）——
    // 在 v1 之上**向后兼容追加**：bincode 按声明顺序给枚举判别号，新变体只追加在末尾，
    // 不影响既有变体编号。旧版本（v0.1.x）收到这些变体会解码失败并断开（优雅降级）。
    /// 发起方握手后请求对端共享索引（仅完全信任设备之间有效）。
    IndexRequest,
    /// 持有方分批回送索引条目；`last = true` 标记最后一批。
    IndexEntries {
        entries: Vec<IndexItem>,
        last: bool,
    },

    // —— 按需拉取（SYNC_DESIGN.md §4 / 里程碑 4）——
    /// 拉取方握手后请求对端某个共享文件（`rel_path` 为统一视图的限定展示路径）。
    /// 对端校验完全信任 + 路径落在共享范围内后，**反转角色**用既有发送流回推该文件
    /// （`Offer` → 分块 → `FileDone`/`FileAck` → `TaskDone`），不新增数据通路。
    FetchRequest {
        rel_path: String,
    },

    // —— 断点续传（PROTOCOL.md §13 / CONNECT_DESIGN.md §5，里程碑 C1）——
    /// 接收方在 `OfferAnswer{accept:true}` 之后**确定性地**紧跟发送（仅双方协商
    /// proto ≥ `RESUME_PROTO_VERSION` 时）：报告每个文件已落盘 `.aa4c-part` 的可信
    /// 续传起点。发送方据此从 `verified_bytes` 处续传，不必重发已确认部分。
    ResumeReport {
        task_id: TaskId,
        progress: Vec<FileProgress>,
    },

    // —— 分享链接（CONNECT_DESIGN.md §7 / PROTOCOL.md §16，里程碑 C6）——
    /// 打开分享链接的一方握手后请求某个 token 对应的内容。**无需 `trusted`**——token
    /// 本身就是访问能力（capability，CONNECT_DESIGN.md §7.1），不要求请求方是已配对设备。
    /// 对端校验 token 有效（未过期、未吊销）后按 `FetchRequest` 同一套「反转角色回推」
    /// 处理（`Offer` → 分块 → `FileDone`/`FileAck` → `TaskDone`），不新增数据通路。
    ShareRequest {
        token: String,
    },

    // —— 配对时交换 server_hint（PROTOCOL.md §17，proto ≥ SERVER_HINT_PROTO_VERSION，
    //    V0.3 遗留 gap 补完）——
    // 同 `ResumeReport`：**不修改既有 `PairRequest`/`PairAccept`**（它们携带的 `DeviceInfo`
    // 是 bincode 位置编码的既有结构体，追加字段会破坏所有旧版本客户端的解码），改为追加
    // 变体，双方在 `PairConfirm`/`PairConfirm` 互相确认之后、写库之前确定性交换。
    /// 各自声明自己当前配置的 home server 地址（`enable_remote` 关闭或未配置时为
    /// `None`），供对端以后跨服务器好友寻址时去查自己（见 `aa4c-core::orchestrate::
    /// resolve_addr`）。proto < `SERVER_HINT_PROTO_VERSION` 时两端都不发送，不认识
    /// 这条消息，行为与旧版完全一致。
    PairServerHint {
        server_hint: Option<String>,
    },

    // —— 信任传递 / 引荐（TRUST_DESIGN.md §5，PROTOCOL.md §18，
    //    proto ≥ INTRODUCE_PROTO_VERSION，里程碑 R2）——
    /// 请求对端列出「它认为也属于同一个人」的设备（仅完全信任设备之间有效）。
    /// 与 `IndexRequest` 同形：请求方握手后发出，持有方分批回送。
    IntroduceRequest,
    /// 持有方分批回送引荐条目；`last = true` 标记最后一批。
    ///
    /// **只回送完全信任（`full`）的设备**——`friend` 是「别人的设备」，把它引荐给自己的
    /// 另一台设备等于替用户扩大信任范围，不做（TRUST_DESIGN.md §5.3）。
    IntroducePeers {
        peers: Vec<PeerIntro>,
        last: bool,
    },
}

/// 引荐条目（`IntroducePeers` 载荷，TRUST_DESIGN.md §5.5）。
///
/// 携带公钥而非仅指纹：`device_id == BLAKE3(public_key)`（见 `aa4c_identity::
/// device_id_from_public_key`），收方**本地即可校验两者自洽**，恶意引荐者无法递来一个
/// 与公钥对不上的指纹；同时补齐 `devices.public_key`（NOT NULL）这一列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIntro {
    /// 被引荐设备的指纹。
    pub device_id: DeviceId,
    /// 被引荐设备的 Ed25519 公钥原始字节（32 字节）。
    pub public_key: Vec<u8>,
    /// 展示名与平台，仅供用户在确认界面辨认，不参与任何信任判定。
    pub name: String,
    pub platform: String,
    /// 引荐者已知的对端 home server 地址（`aa4c://host:port#fp`），可空。
    pub server_hint: Option<String>,
}

/// 断点续传进度条目（`ResumeReport` 载荷，PROTOCOL.md §13）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileProgress {
    pub file_index: u32,
    /// 可信续传起点（字节偏移）；发送方从此处继续，接收方从此处继续写入。
    pub verified_bytes: u64,
}

/// 编码为完整帧（长度前缀 + 消息体）。泛型化以复用给 `ServerMessage`（`server` 子模块，
/// 里程碑 C2）——帧格式对两套协议完全相同，只是消息类型不同。
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>> {
    let body = bincode::serialize(msg).map_err(|e| Aa4cError::Protocol(format!("encode: {e}")))?;
    if body.len() > MAX_FRAME_LEN {
        return Err(Aa4cError::Protocol(format!(
            "frame too large: {} bytes",
            body.len()
        )));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(
        &u32::try_from(body.len())
            .expect("checked above")
            .to_be_bytes(),
    );
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// 解码消息体（不含长度前缀）。
pub fn decode_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T> {
    bincode::deserialize(body).map_err(|e| Aa4cError::Protocol(format!("decode: {e}")))
}

/// 写出一条消息并 flush。
pub async fn write_message<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    msg: &T,
) -> Result<()> {
    let frame = encode_frame(msg)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// 读取一条消息。超长帧立即报错（上层应断开连接）。
pub async fn read_message<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> Result<T> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(Aa4cError::Protocol(format!("frame too large: {len} bytes")));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    decode_body(&body)
}

/// 发起方握手：写 Hello，读 HelloAck，返回（对端声明的 device_id, 协商协议版本）。
///
/// 调用方必须校验返回的 device_id 与 TLS 证书指纹一致（PROTOCOL.md §5）。
pub async fn client_hello<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    self_id: &DeviceId,
) -> Result<(DeviceId, u16)> {
    write_message(
        stream,
        &Message::Hello {
            proto: PROTO_VERSION,
            device_id: self_id.clone(),
        },
    )
    .await?;
    match read_message(stream).await? {
        Message::HelloAck { proto, device_id } => Ok((device_id, proto.min(PROTO_VERSION))),
        other => Err(unexpected(&other)),
    }
}

/// 接收方握手：读 Hello，写 HelloAck，返回（对端声明的 device_id, 协商协议版本）。
pub async fn server_hello<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    self_id: &DeviceId,
) -> Result<(DeviceId, u16)> {
    let (peer, proto) = match read_message(stream).await? {
        Message::Hello { proto, device_id } => (device_id, proto.min(PROTO_VERSION)),
        other => return Err(unexpected(&other)),
    };
    write_message(
        stream,
        &Message::HelloAck {
            proto: PROTO_VERSION,
            device_id: self_id.clone(),
        },
    )
    .await?;
    Ok((peer, proto))
}

/// 统一的"意外消息"错误（不在错误信息中泄露消息内容，只给变体名）。
pub fn unexpected(msg: &Message) -> Aa4cError {
    let variant = match msg {
        Message::Hello { .. } => "Hello",
        Message::HelloAck { .. } => "HelloAck",
        Message::PairRequest { .. } => "PairRequest",
        Message::PairAccept { .. } => "PairAccept",
        Message::PairConfirm => "PairConfirm",
        Message::PairReject { .. } => "PairReject",
        Message::Offer { .. } => "Offer",
        Message::OfferAnswer { .. } => "OfferAnswer",
        Message::Chunk { .. } => "Chunk",
        Message::FileDone { .. } => "FileDone",
        Message::FileAck { .. } => "FileAck",
        Message::TaskDone { .. } => "TaskDone",
        Message::Cancel { .. } => "Cancel",
        Message::IndexRequest => "IndexRequest",
        Message::IndexEntries { .. } => "IndexEntries",
        Message::FetchRequest { .. } => "FetchRequest",
        Message::ResumeReport { .. } => "ResumeReport",
        Message::ShareRequest { .. } => "ShareRequest",
        Message::PairServerHint { .. } => "PairServerHint",
        Message::IntroduceRequest => "IntroduceRequest",
        Message::IntroducePeers { .. } => "IntroducePeers",
    };
    Aa4cError::Protocol(format!("unexpected message: {variant}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_types::Platform;

    fn sample_device() -> DeviceInfo {
        DeviceInfo {
            id: "ab".repeat(32),
            name: "客厅电脑".into(),
            platform: Platform::Linux,
            version: "0.1.0".into(),
            addr: Some("192.168.1.7:42420".parse().unwrap()),
            online: true,
            trusted: false,
            trust_level: None,
        }
    }

    #[tokio::test]
    async fn message_roundtrip_over_stream() {
        let samples = vec![
            Message::Hello {
                proto: 1,
                device_id: "ab".repeat(32),
            },
            Message::PairRequest {
                device: sample_device(),
                public_key: [7u8; 32],
            },
            Message::PairConfirm,
            Message::Offer {
                task_id: "t1".into(),
                files: vec![FileMeta {
                    rel_path: "照片/IMG (1).jpg".into(),
                    size: 123,
                }],
            },
            Message::Chunk {
                file_index: 3,
                offset: 4 * 1024 * 1024,
                len: 1024,
            },
            Message::Cancel {
                task_id: "t1".into(),
                reason: "user".into(),
            },
        ];
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        for msg in &samples {
            write_message(&mut a, msg).await.unwrap();
            let got = read_message::<_, Message>(&mut b).await.unwrap();
            assert_eq!(&got, msg);
        }
    }

    #[tokio::test]
    async fn read_rejects_oversized_frame_header() {
        // 伪造超过 16 MiB 的长度前缀：必须在分配/读取 body 之前拒绝
        let (mut a, mut b) = tokio::io::duplex(64);
        let len = (MAX_FRAME_LEN as u32) + 1;
        tokio::io::AsyncWriteExt::write_all(&mut a, &len.to_be_bytes())
            .await
            .unwrap();
        let err = read_message::<_, Message>(&mut b).await.unwrap_err();
        assert!(err.to_string().contains("frame too large"), "{err}");
    }

    #[tokio::test]
    async fn read_fails_cleanly_on_truncated_body() {
        let (mut a, mut b) = tokio::io::duplex(64);
        // 声明 10 字节 body 但只发 3 字节后关闭
        tokio::io::AsyncWriteExt::write_all(&mut a, &10u32.to_be_bytes())
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut a, &[1, 2, 3])
            .await
            .unwrap();
        drop(a);
        assert!(read_message::<_, Message>(&mut b).await.is_err());
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_body::<Message>(&[0xff; 16]).is_err());
    }

    #[tokio::test]
    async fn hello_handshake_negotiates_proto() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let id_a = "aa".repeat(32);
        let id_b = "bb".repeat(32);
        let (client, server) =
            tokio::join!(client_hello(&mut a, &id_a), server_hello(&mut b, &id_b));
        let (peer_of_a, proto_a) = client.unwrap();
        let (peer_of_b, proto_b) = server.unwrap();
        assert_eq!(peer_of_a, id_b);
        assert_eq!(peer_of_b, id_a);
        assert_eq!(proto_a, PROTO_VERSION);
        assert_eq!(proto_b, PROTO_VERSION);
    }

    #[tokio::test]
    async fn resume_report_roundtrips_and_appends_after_existing_variants() {
        // 追加变体不影响既有编号：本测试与 message_roundtrip_over_stream 用同一批旧变体
        // 混合编解码，任何一个变体的判别号被打乱都会在这里炸出来。
        let samples = vec![
            Message::Offer {
                task_id: "t1".into(),
                files: vec![FileMeta {
                    rel_path: "a.bin".into(),
                    size: 10,
                }],
            },
            Message::ResumeReport {
                task_id: "t1".into(),
                progress: vec![
                    FileProgress {
                        file_index: 0,
                        verified_bytes: 4 * 1024 * 1024,
                    },
                    FileProgress {
                        file_index: 1,
                        verified_bytes: 0,
                    },
                ],
            },
            Message::Cancel {
                task_id: "t1".into(),
                reason: "done".into(),
            },
        ];
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        for msg in &samples {
            write_message(&mut a, msg).await.unwrap();
            let got = read_message::<_, Message>(&mut b).await.unwrap();
            assert_eq!(&got, msg);
        }
    }

    #[tokio::test]
    async fn share_request_roundtrips_and_appends_after_existing_variants() {
        // 同 resume_report 的追加变体回归测试：混合旧变体编解码，判别号被打乱会在这里炸出来。
        let samples = vec![
            Message::Offer {
                task_id: "t1".into(),
                files: vec![FileMeta {
                    rel_path: "a.bin".into(),
                    size: 10,
                }],
            },
            Message::ShareRequest {
                token: "9WsBRcPzM5efFqZmYBcJExZzn6mzybf2mBGoDDzMWK5H".into(),
            },
            Message::Cancel {
                task_id: "t1".into(),
                reason: "invalid_or_expired_token".into(),
            },
        ];
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        for msg in &samples {
            write_message(&mut a, msg).await.unwrap();
            let got = read_message::<_, Message>(&mut b).await.unwrap();
            assert_eq!(&got, msg);
        }
    }

    #[tokio::test]
    async fn client_hello_negotiates_down_to_v1_peer() {
        // 模拟老版本（v1）对端：它只回 HelloAck{proto:1}。发起方应把协商版本降到 1，
        // 上层据此跳过 v2 的索引/拉取消息（优雅降级，PROTOCOL.md §8b / §14）。
        let (mut a, mut b) = tokio::io::duplex(1024);
        let id_a = "aa".repeat(32);
        let v1_peer = async {
            // 读 Hello，回一个 proto=1 的 HelloAck（不管本机实际版本）
            let _ = read_message::<_, Message>(&mut b).await.unwrap();
            write_message(
                &mut b,
                &Message::HelloAck {
                    proto: 1,
                    device_id: "bb".repeat(32),
                },
            )
            .await
            .unwrap();
        };
        let (client, ()) = tokio::join!(client_hello(&mut a, &id_a), v1_peer);
        let (_peer, proto) = client.unwrap();
        assert_eq!(proto, 1, "negotiated down to v1");
        assert!(
            proto < aa4c_types::SYNC_PROTO_VERSION,
            "gate would skip sync"
        );
    }
}
