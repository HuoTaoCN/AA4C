//! 归档类型（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md §2/§4）。
//!
//! 类别体系内置固定（标签才是用户的自由维度，见 ARCHIVE_DESIGN.md §2.1）；规则的匹配
//! 条件/动作用结构体表示，落库时整体 `serde_json` 成 `match_json`/`action_json` 两列
//! TEXT（字段组合随规则形态变化，同 `settings` KV 的一贯"不拆大量可空列"做法）。

use serde::{Deserialize, Serialize};

/// 内置类别（不可增删）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveCategory {
    Model,
    Image,
    Video,
    Audio,
    Document,
    Ebook,
    Archive,
    Installer,
    Code,
    Subtitle,
    Other,
}

impl ArchiveCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Ebook => "ebook",
            Self::Archive => "archive",
            Self::Installer => "installer",
            Self::Code => "code",
            Self::Subtitle => "subtitle",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for ArchiveCategory {
    type Err = crate::Aa4cError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "model" => Ok(Self::Model),
            "image" => Ok(Self::Image),
            "video" => Ok(Self::Video),
            "audio" => Ok(Self::Audio),
            "document" => Ok(Self::Document),
            "ebook" => Ok(Self::Ebook),
            "archive" => Ok(Self::Archive),
            "installer" => Ok(Self::Installer),
            "code" => Ok(Self::Code),
            "subtitle" => Ok(Self::Subtitle),
            "other" => Ok(Self::Other),
            other => Err(crate::Aa4cError::Protocol(format!(
                "unknown archive category: {other}"
            ))),
        }
    }
}

/// 规则匹配条件（ARCHIVE_DESIGN.md §2.3）。`categories` 为空集合视为"任意类别都匹配"。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMatch {
    pub categories: Vec<ArchiveCategory>,
    pub extensions: Option<Vec<String>>,
    pub glob: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
}

/// 规则动作：目标目录模板（占位符 `{类别}`/`{年}`/`{月}`/`{扩展名}`/`{模型.架构}`/
/// `{模型.名称}`/`{模型.量化}`，见 ARCHIVE_DESIGN.md §2.3）+ 命中后追加的标签。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveAction {
    pub target_template: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 一条归档规则（`archive_rules` 表一行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// 匹配顺序，取第一条命中的规则（不叠加）。
    pub position: i64,
    pub matcher: ArchiveMatch,
    pub action: ArchiveAction,
    pub created_at: i64,
    pub updated_at: i64,
}

/// GGUF 头解析出的模型元数据（ARCHIVE_DESIGN.md §2.2），仅「模型」类别的
/// `ArchiveEntry` 非 `None`。字段取不到值时为 `None`，不是解析失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelMeta {
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub size_label: Option<String>,
    pub file_type: Option<String>,
    pub context_length: Option<u64>,
}

/// 一条被归档引擎移动/纳管的文件记录（`archive_entries` 表一行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub id: String,
    pub current_path: String,
    pub category: ArchiveCategory,
    pub size: u64,
    pub model_meta: Option<ModelMeta>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 标签来源：规则自动追加 / AI 建议采纳 / 用户手动打（ARCHIVE_DESIGN.md §0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagSource {
    Rule,
    Ai,
    User,
}

impl TagSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Ai => "ai",
            Self::User => "user",
        }
    }
}

impl std::str::FromStr for TagSource {
    type Err = crate::Aa4cError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rule" => Ok(Self::Rule),
            "ai" => Ok(Self::Ai),
            "user" => Ok(Self::User),
            other => Err(crate::Aa4cError::Protocol(format!(
                "unknown tag source: {other}"
            ))),
        }
    }
}

/// 一条归档标签（`archive_tags` 表一行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveTag {
    pub entry_id: String,
    pub tag: String,
    pub source: TagSource,
}

/// 一条移动历史（`archive_log` 表一行），供撤销（ARCHIVE_DESIGN.md §2.4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveLogEntry {
    pub id: i64,
    pub entry_id: String,
    pub from_path: String,
    pub to_path: String,
    /// `None` = 手动归档（不是某条规则触发的）。
    pub rule_id: Option<String>,
    pub at: i64,
    pub undone: bool,
}
