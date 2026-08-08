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

/// 设备信任分级（DATABASE_SCHEMA §4 / SYNC_DESIGN §2）。
///
/// 只有已配对设备入库，故库内只有这两级；"临时"不入库、"陌生"仅在内存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    /// 完全信任：自己的多台设备，参与跨设备索引 / 同步。
    Full,
    /// 朋友 / 家庭 / 团队：仅收发与手动分享，不参与同步。
    Friend,
}

impl TrustLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Friend => "friend",
        }
    }
}

impl FromStr for TrustLevel {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "full" => Ok(Self::Full),
            "friend" => Ok(Self::Friend),
            other => Err(Aa4cError::Protocol(format!("invalid trust level: {other}"))),
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
    /// 信任分级；未配对（仅发现）的设备为 `None`。
    pub trust_level: Option<TrustLevel>,
}

/// 待用户确认的引荐（TRUST_DESIGN.md §5.4，里程碑 R2）。
///
/// 「某台你已经完全信任的设备说，这台也是你的」。设计上**刻意不自动信任**：这里的每一
/// 条都要用户点一次确认才会变成已配对设备（Syncthing 式自动引荐有传递失控与「删了又被
/// 加回来」两个已记录在案的坑，见 TRUST_DESIGN.md §5.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingIntroduction {
    /// 被引荐设备的指纹。
    pub device_id: DeviceId,
    pub name: String,
    pub platform: Platform,
    /// 引荐者的指纹，以及它在本机的展示名（引荐者已被删除时为 `None`）。
    /// UI 必须把它显示出来——用户判断「要不要信」靠的就是「谁说的」。
    pub introduced_by: DeviceId,
    pub introduced_by_name: Option<String>,
    /// 首次收到这条引荐的时间（unix 毫秒）。
    pub introduced_at: i64,
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
            trust_level: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["platform"], "macos");
        assert_eq!(json["addr"], "192.168.1.2:42420");
        assert_eq!(json["trustLevel"], serde_json::Value::Null);
        let back: DeviceInfo = serde_json::from_value(json).unwrap();
        assert_eq!(back, info);
    }
}
