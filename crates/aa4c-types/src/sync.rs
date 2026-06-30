//! 同步范围与本机文件索引类型（SYNC_DESIGN.md §3 / §6，DATABASE_SCHEMA.md §4.2-4.3）。
//!
//! V0.2 里程碑 2：只有本机扫描出的条目（`present_local` 恒为 true）；
//! 跨设备摘要交换、`remote_index`、黄/红状态留给后续里程碑。

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Aa4cError, Result};

/// 共享范围种类：用户选定的同步文件夹，或固定的「收到的」（Inbox，自动维护）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    Folder,
    Inbox,
}

impl ScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Inbox => "inbox",
        }
    }
}

impl FromStr for ScopeKind {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "folder" => Ok(Self::Folder),
            "inbox" => Ok(Self::Inbox),
            other => Err(Aa4cError::Protocol(format!("invalid scope kind: {other}"))),
        }
    }
}

/// 共享范围（DATABASE_SCHEMA.md §4.2 `sync_scopes`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncScope {
    pub id: String,
    pub kind: ScopeKind,
    /// 本机绝对路径。
    pub local_path: String,
    pub created_at: i64,
}

/// 本机文件索引条目（DATABASE_SCHEMA.md §4.3 `sync_file_index`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFileEntry {
    pub scope_id: String,
    /// 范围内相对路径，`/` 分隔。
    pub rel_path: String,
    pub size: u64,
    /// unix 毫秒。
    pub mtime: i64,
    /// BLAKE3 hex；惰性计算，mtime/size 未变时复用旧值。
    pub hash: Option<String>,
    /// 内容是否在本机磁盘（V0.2 里程碑 2 恒为 true，本机扫描出的条目）。
    pub present_local: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_kind_str_roundtrip() {
        for k in [ScopeKind::Folder, ScopeKind::Inbox] {
            assert_eq!(k.as_str().parse::<ScopeKind>().unwrap(), k);
        }
        assert!("bogus".parse::<ScopeKind>().is_err());
    }

    #[test]
    fn sync_file_entry_json_is_camel_case() {
        let e = SyncFileEntry {
            scope_id: "s1".into(),
            rel_path: "照片/a.jpg".into(),
            size: 100,
            mtime: 1_750_000_000_000,
            hash: Some("ab".repeat(32)),
            present_local: true,
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["scopeId"], "s1");
        assert_eq!(json["relPath"], "照片/a.jpg");
        assert_eq!(json["presentLocal"], true);
        let back: SyncFileEntry = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
    }
}
