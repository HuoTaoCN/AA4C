//! 分享链接（AA Share）类型 + 链接编解码（CONNECT_DESIGN.md §7/§8，里程碑 C6）。
//!
//! 链接格式：`aa4c://share/<base58(payload)>`。payload 是三个字段（分享方 device_id、
//! token、分享方当时配置的服务器地址）序列化成 JSON 后整体 base58——选 JSON 而非手写
//! 分隔符格式纯粹图省事（这几个字段没有性能敏感场景，JSON 免去转义心智负担），base58
//! 只是让链接本身好看/好传播（不含容易看混的 `0`/`O`/`I`/`l`）。payload **不含内容、
//! 不含密钥**，配套二维码留待移动端里程碑（CONNECT_DESIGN.md §7.2）。

use serde::{Deserialize, Serialize};

use crate::{Aa4cError, DeviceId, Result};

const LINK_PREFIX: &str = "aa4c://share/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SharePayload {
    host_id: DeviceId,
    token: String,
    host_server: Option<String>,
}

/// 已解析 / 待编码的分享链接内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLink {
    pub host_id: DeviceId,
    pub token: String,
    /// 分享方当时配置的服务器地址（`aa4c://host:port#fp`），未配置为 `None`——同一局域网
    /// 可不靠它（mDNS 直接找到 host），远程可达随连接阶梯就绪自然生效。
    pub host_server: Option<String>,
}

impl ShareLink {
    pub fn encode(&self) -> String {
        let payload = SharePayload {
            host_id: self.host_id.clone(),
            token: self.token.clone(),
            host_server: self.host_server.clone(),
        };
        let json = serde_json::to_vec(&payload).expect("SharePayload always serializes");
        format!("{LINK_PREFIX}{}", bs58::encode(json).into_string())
    }

    pub fn parse(link: &str) -> Result<Self> {
        let encoded = link.strip_prefix(LINK_PREFIX).ok_or_else(|| bad(link))?;
        let bytes = bs58::decode(encoded).into_vec().map_err(|_| bad(link))?;
        let payload: SharePayload = serde_json::from_slice(&bytes).map_err(|_| bad(link))?;
        if payload.host_id.is_empty() || payload.token.is_empty() {
            return Err(bad(link));
        }
        Ok(Self {
            host_id: payload.host_id,
            token: payload.token,
            host_server: payload.host_server,
        })
    }
}

fn bad(s: &str) -> Aa4cError {
    Aa4cError::Protocol(format!("invalid share link: {s:?}"))
}

/// 一条分享记录（`shares` 表一行 + Core 现算的完整链接）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub id: String,
    pub token: String,
    /// 被分享的限定路径（统一视图口径，须落在某个共享范围内）。
    pub rel_path: String,
    /// V0.3 首版恒为 `"read"`（`"readwrite"` 留字段余量，未实现）。
    pub permission: String,
    /// 绝对过期时间（unix 毫秒）；`None` = 长期有效。
    pub expires_at: Option<i64>,
    /// `"open"` | `"revoked"`。
    pub status: String,
    pub created_at: i64,
    /// 完整可分享链接（`aa4c://share/...`）。**由 Core 在读出 DB 行后现算**——DB 只存
    /// `token`，组装链接还需要本机 `device_id` + 当前配置的服务器地址，这些不是
    /// `shares` 表的字段（`aa4c-store` 返回的行此字段为空字符串，见 `Store::list_shares`）。
    pub link: String,
}

/// 一条分享访问记录（`share_access` 表一行，供「查看访问记录」，里程碑 C6 可选功能）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareAccess {
    pub id: i64,
    pub share_id: String,
    /// 访问方 device_id；匿名访问为 `None`（V0.3 无此路径，mTLS 握手总能读出证书身份，
    /// 留字段余量呼应 `share_access.peer_id` 的 nullable 设计）。
    pub peer_id: Option<DeviceId>,
    /// `"list"` | `"download"` | `"upload"`。
    pub action: String,
    pub at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_round_trips() {
        let link = ShareLink {
            host_id: "a".repeat(64),
            token: "tok123".to_string(),
            host_server: Some("aa4c://example.com:42420#abcd1234abcd1234".to_string()),
        };
        let encoded = link.encode();
        assert!(encoded.starts_with("aa4c://share/"));
        let back = ShareLink::parse(&encoded).unwrap();
        assert_eq!(back, link);
    }

    #[test]
    fn link_round_trips_without_host_server() {
        let link = ShareLink {
            host_id: "b".repeat(64),
            token: "tok456".to_string(),
            host_server: None,
        };
        let back = ShareLink::parse(&link.encode()).unwrap();
        assert_eq!(back, link);
    }

    #[test]
    fn rejects_malformed_links() {
        for bad in [
            "http://share/abc",     // 错误 scheme
            "aa4c://share/",        // 空 payload
            "aa4c://share/00000",   // 不是合法 base58（含 0）
            "aa4c://share/notb58!", // 非法字符
        ] {
            assert!(ShareLink::parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
