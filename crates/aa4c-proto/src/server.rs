//! `aa4c-server` 信令协议（PROTOCOL.md Part C，CONNECT_DESIGN.md §3.2，里程碑 C2）。
//!
//! 独立于设备间 [`crate::Message`] 的消息族，独立版本号 [`SERVER_PROTO_VERSION`]，
//! 同样遵守「只追加变体」纪律。复用 [`crate::encode_frame`] / [`crate::read_message`] 等
//! 已泛型化的帧层，帧格式（4 字节大端长度 + bincode）与设备间协议完全相同。
//!
//! **身份验证复用 mTLS**，不单独实现设计稿里的 `Challenge`/`ChallengeReply`：客户端与
//! 服务器的连接和设备间传输一样走 mTLS（服务器接受任意合法 Ed25519 客户端证书，见
//! `aa4c_identity::tls_server_config(None)`），握手完成的那一刻 TLS 已经密码学证明了
//! 对端持有其证书对应的私钥——应用层再做一次签名挑战是重复劳动，且需要额外引入独立于
//! TLS 的签名依赖。这是本里程碑对 CONNECT_DESIGN 初稿的收敛，安全属性等价（甚至更强，
//! 因为身份绑定在整条连接上而非单条消息）。
//!
//! C2 只定义信令所需变体（`SrvHello`/`Register`/`Lookup`）；`Signal`/`RelayRequest`/
//! `RelayGrant` 留到 C3/C5 按同样的「只追加」纪律加入。

use std::net::SocketAddr;

use aa4c_types::{Aa4cError, DeviceId};
use serde::{Deserialize, Serialize};

/// 信令协议版本，独立于设备间 `PROTO_VERSION` 编号。
pub const SERVER_PROTO_VERSION: u16 = 1;

/// `aa4c-server` 信令消息（PROTOCOL.md Part C）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// 客户端握手：声明自身支持的信令协议版本。
    SrvHello { server_proto: u16 },
    /// 服务器握手应答：协商版本（取双方最小值）。
    SrvHelloAck { server_proto: u16 },

    /// 注册候选端点 + 当前已配对设备允许名单；周期性续约覆盖旧登记（TTL 由服务器决定，
    /// 通过 `RegisterAck.ttl_secs` 告知）。查询/注册方身份由 mTLS 证书确定，不在消息里
    /// 重复携带。
    Register {
        /// 候选端点（自报告的本机可达地址）；服务器会额外记录本连接的观测源地址。
        endpoints: Vec<SocketAddr>,
        /// 设备当前协议版本（供未来细粒度能力协商参考，本里程碑仅记录不使用）。
        proto: u16,
        /// 当前已配对设备 id 列表——查询授权的唯一依据（CONNECT_DESIGN.md §3.3）。
        allow_list: Vec<DeviceId>,
    },
    /// 注册确认：`ttl_secs` 是本次登记的有效期，客户端应显著早于此间隔重新注册。
    RegisterAck { ttl_secs: u64 },

    /// 查询目标设备当前端点。
    Lookup { device_id: DeviceId },
    /// 查询结果：未注册 / 已过期 / 查询方不在目标允许名单内一律回空列表，不区分原因
    /// （防止以此探测 DeviceId 是否存在，PROTOCOL.md §15）。
    LookupReply { endpoints: Vec<SocketAddr> },
}

/// 统一的"意外消息"错误（只给变体名，不泄露 payload，呼应 [`crate::unexpected`]）。
pub fn unexpected(msg: &ServerMessage) -> Aa4cError {
    let variant = match msg {
        ServerMessage::SrvHello { .. } => "SrvHello",
        ServerMessage::SrvHelloAck { .. } => "SrvHelloAck",
        ServerMessage::Register { .. } => "Register",
        ServerMessage::RegisterAck { .. } => "RegisterAck",
        ServerMessage::Lookup { .. } => "Lookup",
        ServerMessage::LookupReply { .. } => "LookupReply",
    };
    Aa4cError::Protocol(format!("unexpected server message: {variant}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read_message, write_message};

    #[tokio::test]
    async fn server_message_roundtrips_over_stream() {
        let samples = vec![
            ServerMessage::SrvHello {
                server_proto: SERVER_PROTO_VERSION,
            },
            ServerMessage::SrvHelloAck {
                server_proto: SERVER_PROTO_VERSION,
            },
            ServerMessage::Register {
                endpoints: vec!["203.0.113.5:42420".parse().unwrap()],
                proto: aa4c_types::PROTO_VERSION,
                allow_list: vec!["aa".repeat(32), "bb".repeat(32)],
            },
            ServerMessage::RegisterAck { ttl_secs: 60 },
            ServerMessage::Lookup {
                device_id: "aa".repeat(32),
            },
            ServerMessage::LookupReply { endpoints: vec![] },
        ];
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        for msg in &samples {
            write_message(&mut a, msg).await.unwrap();
            let got: ServerMessage = read_message(&mut b).await.unwrap();
            assert_eq!(&got, msg);
        }
    }

    #[test]
    fn unexpected_does_not_leak_payload() {
        let err = unexpected(&ServerMessage::Lookup {
            device_id: "supersecretdeviceid".into(),
        });
        assert!(!err.to_string().contains("supersecretdeviceid"));
        assert!(err.to_string().contains("Lookup"));
    }
}
