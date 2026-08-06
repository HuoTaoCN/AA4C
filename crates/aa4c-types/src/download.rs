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

/// 一条下载任务的自定义选项（对标 Motrix 新建任务对话框的"高级选项"，
/// DOWNLOAD_DESIGN.md §5/§10 预留的"引擎无关请求描述"）。全部字段可选，
/// 全为 `None` 时整体不落库（`download_tasks.options` 为 NULL）。
///
/// 落库是为了让 `retry()` 不丢这些选项——HTTP 任务的重试是"删旧记录 + 用原 URL
/// 重新添加"，不存的话 referer/cookie 全丢，而这恰恰是重试最需要的东西
/// （见迁移 011 的注释）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOptions {
    /// 这条任务单独的保存目录，`None` = 用全局下载目录。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub save_dir: Option<String>,
    /// 自定义保存文件名，`None` = 用服务器/种子给的名字。BT 任务不适用。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub out: Option<String>,
    /// 来源页地址——防盗链站点会校验它，不带就 403。BT 任务不适用。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub referer: Option<String>,
    /// Cookie，用于需要登录态才能下的直链。BT 任务不适用。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cookie: Option<String>,
}

impl DownloadOptions {
    /// 一个字段都没填 —— 调用方据此决定不落库（绝大多数任务都是这种）。
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
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
    /// 这条任务的自定义选项（迁移 011）；`None` = 没有任何自定义选项。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<DownloadOptions>,
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
            options: None,
        };
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["savePath"], serde_json::Value::Null);
        assert_eq!(json["totalBytes"], 1000);
        assert_eq!(json["downloadedBytes"], 200);
        assert_eq!(json["status"], "active");
        assert_eq!(json["kind"], "http");
        // 没有自定义选项时这个 key 直接不出现（`skip_serializing_if`），不是
        // 出现成 null——同 `DownloadProgress` 里 BT 专属字段的既有取舍。
        assert!(json.get("options").is_none());
    }

    #[test]
    fn download_options_json_is_camel_case_and_omits_empty_fields() {
        let opts = DownloadOptions {
            save_dir: Some("/tmp/x".into()),
            referer: Some("https://example.com/page".into()),
            ..DownloadOptions::default()
        };
        assert!(!opts.is_empty());
        assert!(DownloadOptions::default().is_empty());

        let json = serde_json::to_value(&opts).unwrap();
        assert_eq!(json["saveDir"], "/tmp/x");
        assert_eq!(json["referer"], "https://example.com/page");
        assert!(json.get("out").is_none());
        assert!(json.get("cookie").is_none());

        let back: DownloadOptions = serde_json::from_value(json).unwrap();
        assert_eq!(back, opts);
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
