//! 自建 `aa4c-server` 地址解析（CONNECT_DESIGN.md §3.1，PROTOCOL.md Part C，里程碑 C2）。
//!
//! 格式：`aa4c://host:port#<证书指纹前缀 hex>`。指纹通常取服务器 DeviceId
//! （BLAKE3(公钥) hex）的前 16 位，写死在地址里即拿到信任锚点——连接后从对端证书
//! 算出真实 DeviceId，比对是否以此前缀开头，不一致立即拒绝（无 TOFU 窗口）。

use serde::{Deserialize, Serialize};

use crate::{Aa4cError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddr {
    pub host: String,
    pub port: u16,
    /// 小写 hex 前缀（通常 16 位，但只要求非空 + 合法 hex 字符，不强制长度）。
    pub fingerprint_prefix: String,
}

impl ServerAddr {
    pub fn parse(s: &str) -> Result<Self> {
        let rest = s.strip_prefix("aa4c://").ok_or_else(|| bad(s))?;
        let (hostport, fp) = rest.split_once('#').ok_or_else(|| bad(s))?;
        // IPv6 走 URL 惯例的方括号形式（`aa4c://[2408:8000::1]:42421#fp`），**括号要剥掉**
        // 再存：`host` 最终是拿去做 `(host, port)` 的 `ToSocketAddrs` 的，带括号会被当成
        // 域名去解析而失败。不带括号则无法与端口分隔符区分（IPv6 本身满是冒号）——所以
        // 两件事必须成对做（里程碑 R1/R4）。
        let (host, port) = match hostport.strip_prefix('[') {
            Some(rest6) => {
                let (h, p) = rest6.split_once("]:").ok_or_else(|| bad(s))?;
                (h, p)
            }
            None => hostport.rsplit_once(':').ok_or_else(|| bad(s))?,
        };
        let port: u16 = port.parse().map_err(|_| bad(s))?;
        if host.is_empty() || fp.is_empty() || !fp.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad(s));
        }
        Ok(Self {
            host: host.to_string(),
            port,
            fingerprint_prefix: fp.to_lowercase(),
        })
    }
}

/// 内置服务器地址里的 `host` 是从哪来的（TRUST_DESIGN.md §6.3，里程碑 R4）。
///
/// 这一项决定了那个地址**到底能不能跨网用**，界面上必须如实说，不能给用户一个看着像
/// 能用、实际出了这个网就废的地址。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalServerReach {
    /// 用户自己填的域名 / 固定地址。**唯一真正可靠的一种**——这正是 §6.3 说的
    /// 「你仍然需要一个稳定入口」。
    Configured,
    /// 自动探到的公网地址（本机的公网 IPv6，或 UPnP 映射拿到的公网 IPv4）。
    /// 现在能用，但家宽的地址会变，变了这个链接就失效。
    Detected,
    /// 只找到局域网地址。**出了这个局域网就没用**。
    LanOnly,
}

/// 内置服务器当前状态（里程碑 R4），供设置页展示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalServerStatus {
    /// 是否真的在跑（设置打开了但端口被占也会是 false）。
    pub running: bool,
    /// 实际监听端口；没在跑时为设置里配置的那个。
    pub port: u16,
    /// 给别的设备填的完整地址 `aa4c://host:port#指纹`；没在跑时为 `None`。
    pub address: Option<String>,
    /// 上面那个地址里的 host 是怎么来的。
    pub reach: LocalServerReach,
}

fn bad(s: &str) -> Aa4cError {
    Aa4cError::Protocol(format!("invalid server address: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_address() {
        let a = ServerAddr::parse("aa4c://example.com:42420#ABCD1234abcd1234").unwrap();
        assert_eq!(a.host, "example.com");
        assert_eq!(a.port, 42420);
        assert_eq!(a.fingerprint_prefix, "abcd1234abcd1234"); // 大小写归一
    }

    #[test]
    fn parses_ip_host() {
        let a = ServerAddr::parse("aa4c://127.0.0.1:42430#deadbeef").unwrap();
        assert_eq!(a.host, "127.0.0.1");
        assert_eq!(a.port, 42430);
    }

    #[test]
    fn parses_bracketed_ipv6_host() {
        // 方括号必须被剥掉：`host` 是拿去做 `(host, port)` 的 `ToSocketAddrs` 的，
        // 带着括号会被当域名解析而连不上（里程碑 R1/R4）。
        let a = ServerAddr::parse("aa4c://[2408:8000:1234::1]:42421#deadbeef").unwrap();
        assert_eq!(a.host, "2408:8000:1234::1");
        assert_eq!(a.port, 42421);
        // 剥完之后必须真的能当 IP 用
        assert!(a.host.parse::<std::net::IpAddr>().is_ok());

        // 回环 IPv6 同理
        let a = ServerAddr::parse("aa4c://[::1]:42421#abcd").unwrap();
        assert_eq!(a.host, "::1");
    }

    #[test]
    fn rejects_malformed_ipv6() {
        // 有左括号却没有 `]:` 分隔
        assert!(ServerAddr::parse("aa4c://[2408::1:42421#abcd").is_err());
        assert!(ServerAddr::parse("aa4c://[2408::1]42421#abcd").is_err());
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "http://x:1#ab", // 错误 scheme
            "aa4c://x#ab",   // 缺端口
            "aa4c://x:1",    // 缺指纹
            "aa4c://:1#ab",  // 空 host
            "aa4c://x:1#",   // 空指纹
            "aa4c://x:1#zz", // 非 hex 指纹
            "aa4c://x:notaport#ab",
        ] {
            assert!(ServerAddr::parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
