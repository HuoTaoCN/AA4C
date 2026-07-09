//! 下载任务类型（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md §4/§9）。
//!
//! `id` 直接复用下载引擎原生任务号（aria2 GID）——不做二次 UUID 映射，
//! 见 DOWNLOAD_DESIGN.md §3.3/§3.4。

use serde::{Deserialize, Serialize};

use crate::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    Http,
    Bt,
}

impl DownloadKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Bt => "bt",
        }
    }
}

impl std::str::FromStr for DownloadKind {
    type Err = crate::Aa4cError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "http" => Ok(Self::Http),
            "bt" => Ok(Self::Bt),
            other => Err(crate::Aa4cError::Protocol(format!(
                "unknown download kind: {other}"
            ))),
        }
    }
}

/// 引擎六态（aria2 `tellStatus`/事件通知原生状态，DOWNLOAD_DESIGN.md §3.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Active,
    Waiting,
    Paused,
    Error,
    Complete,
    Removed,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Error => "error",
            Self::Complete => "complete",
            Self::Removed => "removed",
        }
    }
}

impl std::str::FromStr for DownloadStatus {
    type Err = crate::Aa4cError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "waiting" => Ok(Self::Waiting),
            "paused" => Ok(Self::Paused),
            "error" => Ok(Self::Error),
            "complete" => Ok(Self::Complete),
            "removed" => Ok(Self::Removed),
            other => Err(crate::Aa4cError::Protocol(format!(
                "unknown download status: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: TaskId,
    pub kind: DownloadKind,
    pub url: String,
    pub save_path: Option<String>,
    pub status: DownloadStatus,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
    /// unix 毫秒。
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_task_json_is_camel_case() {
        let task = DownloadTask {
            id: "2089b8...".into(),
            kind: DownloadKind::Http,
            url: "https://example.com/file.zip".into(),
            save_path: None,
            status: DownloadStatus::Active,
            total_bytes: 1000,
            downloaded_bytes: 200,
            error: None,
            created_at: 0,
        };
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["savePath"], serde_json::Value::Null);
        assert_eq!(json["totalBytes"], 1000);
        assert_eq!(json["downloadedBytes"], 200);
        assert_eq!(json["status"], "active");
        assert_eq!(json["kind"], "http");
    }

    #[test]
    fn download_status_round_trips_through_as_str() {
        for s in [
            DownloadStatus::Active,
            DownloadStatus::Waiting,
            DownloadStatus::Paused,
            DownloadStatus::Error,
            DownloadStatus::Complete,
            DownloadStatus::Removed,
        ] {
            let parsed: DownloadStatus = s.as_str().parse().unwrap();
            assert_eq!(parsed, s);
        }
    }
}
