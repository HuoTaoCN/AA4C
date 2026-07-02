//! 同步范围与本机文件索引类型（SYNC_DESIGN.md §3 / §6，DATABASE_SCHEMA.md §4.2-4.4）。
//!
//! V0.2 里程碑 2 落地了本机索引（`SyncScope` / `SyncFileEntry`）；里程碑 3 在此基础上
//! 加入跨设备索引交换：`RemoteIndexEntry`（远端广播来的条目）与归并后的统一视图
//! `UnifiedFile`（按 `SyncStatus` 着色 🟢/🟡/🔴）。

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
    /// 内容是否在本机磁盘（本机扫描出的条目恒为 true）。
    pub present_local: bool,
}

/// 文件可获取状态（SYNC_DESIGN.md §4）。前端按此着色：
/// 🟢 本地有 / 🟡 可下载（在线设备有）/ 🔴 设备离线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    /// 🟢 本机已有内容（`present_local = 1`）。
    Local,
    /// 🟡 本机没有，但至少一台**在线**完全信任设备有。
    Online,
    /// 🔴 本机没有，且仅**离线**完全信任设备有。
    Offline,
}

/// 远端完全信任设备广播来的索引条目（DATABASE_SCHEMA.md §4.4 `remote_index`）。
///
/// `rel_path` 是**已限定的展示路径**（顶层段为来源分组，如「收到的」或共享文件夹名），
/// 与本机统一视图同命名空间，便于按路径归并（SYNC_DESIGN.md §3.4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteIndexEntry {
    pub device_id: String,
    pub rel_path: String,
    pub size: u64,
    pub hash: Option<String>,
    /// 最近一次收到该条目的时间（unix 毫秒）。
    pub seen_at: i64,
}

/// 统一文件视图条目：本机索引 + 远端索引按 `rel_path`（同名不同 hash 拆分）归并后的结果
/// （SYNC_DESIGN.md §3.4 / §4 / §8）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedFile {
    /// 限定**展示**路径，`/` 分隔；顶层段是来源分组。冲突时按序号区分（`报告 (2).pdf`）。
    pub rel_path: String,
    /// 限定**基准**路径（未加序号，对端认得的真实路径）；拉取时按 `base_path` + `hash` 定位。
    pub base_path: String,
    pub size: u64,
    pub hash: Option<String>,
    pub status: SyncStatus,
    /// 持有此文件的设备名（本机用「这台设备」），可多台。
    pub holders: Vec<String>,
    /// 是否为冲突版本之一（同一 `base_path` 存在多个不同 hash）。
    pub conflict: bool,
}

/// 一条冲突记录（DATABASE_SCHEMA.md §4.5 `sync_conflicts`）：同一 `rel_path`（限定基准路径）
/// 存在多个不同 hash 的版本。里程碑 5 由统一视图实时探测并落库，供人工挑选保留哪份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub rel_path: String,
    pub hash: String,
    pub status: String,
    pub created_at: i64,
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

    #[test]
    fn sync_status_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_value(SyncStatus::Online).unwrap(),
            serde_json::json!("online")
        );
        assert_eq!(
            serde_json::to_value(SyncStatus::Offline).unwrap(),
            serde_json::json!("offline")
        );
    }

    #[test]
    fn unified_file_json_is_camel_case() {
        let f = UnifiedFile {
            rel_path: "收到的/a.jpg".into(),
            base_path: "收到的/a.jpg".into(),
            size: 100,
            hash: Some("ab".repeat(32)),
            status: SyncStatus::Online,
            holders: vec!["客厅电脑".into()],
            conflict: false,
        };
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["relPath"], "收到的/a.jpg");
        assert_eq!(json["basePath"], "收到的/a.jpg");
        assert_eq!(json["status"], "online");
        assert_eq!(json["holders"][0], "客厅电脑");
        assert_eq!(json["conflict"], false);
        let back: UnifiedFile = serde_json::from_value(json).unwrap();
        assert_eq!(back, f);
    }
}
