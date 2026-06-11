//! 设备类型（API_DESIGN.md §3）。

use std::net::SocketAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Aa4cError, Result};

/// 设备 ID = 设备公钥的 BLAKE3 哈希（hex，64 字符）。
pub type DeviceId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
    Android,
    Ios,
    Server,
}

impl Platform {
    /// 数据库与 mDNS TXT 中使用的稳定字符串（DATABASE_SCHEMA.md §2.1）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Server => "server",
        }
    }

    /// 当前编译目标对应的平台。
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else {
            Self::Linux
        }
    }
}

impl FromStr for Platform {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "windows" => Ok(Self::Windows),
            "macos" => Ok(Self::Macos),
            "linux" => Ok(Self::Linux),
            "android" => Ok(Self::Android),
            "ios" => Ok(Self::Ios),
            "server" => Ok(Self::Server),
            other => Err(Aa4cError::Protocol(format!("invalid platform: {other}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: DeviceId,
    /// 用户可见设备名，如 "Huo 的 MacBook"。
    pub name: String,
    pub platform: Platform,
    /// AA4C 版本号。
    pub version: String,
    /// 最近一次发现的地址。
    pub addr: Option<SocketAddr>,
    pub online: bool,
    /// 是否已配对。
    pub trusted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_str_roundtrip() {
        for p in [
            Platform::Windows,
            Platform::Macos,
            Platform::Linux,
            Platform::Android,
            Platform::Ios,
            Platform::Server,
        ] {
            assert_eq!(p.as_str().parse::<Platform>().unwrap(), p);
        }
        assert!("freebsd".parse::<Platform>().is_err());
    }

    #[test]
    fn device_info_json_is_camel_case() {
        let info = DeviceInfo {
            id: "ab".repeat(32),
            name: "测试设备".into(),
            platform: Platform::Macos,
            version: "0.1.0".into(),
            addr: Some("192.168.1.2:42420".parse().unwrap()),
            online: true,
            trusted: false,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["platform"], "macos");
        assert_eq!(json["addr"], "192.168.1.2:42420");
        let back: DeviceInfo = serde_json::from_value(json).unwrap();
        assert_eq!(back, info);
    }
}
