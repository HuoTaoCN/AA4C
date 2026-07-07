//! 自建 `aa4c-server` 地址解析（CONNECT_DESIGN.md §3.1，PROTOCOL.md Part C，里程碑 C2）。
//!
//! 格式：`aa4c://host:port#<证书指纹前缀 hex>`。指纹通常取服务器 DeviceId
//! （BLAKE3(公钥) hex）的前 16 位，写死在地址里即拿到信任锚点——连接后从对端证书
//! 算出真实 DeviceId，比对是否以此前缀开头，不一致立即拒绝（无 TOFU 窗口）。

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
        let (host, port) = hostport.rsplit_once(':').ok_or_else(|| bad(s))?;
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
