//! 本地知识库类型（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6/§4h）。
//!
//! `KbChunk`（含 embedding 向量）是纯内部检索用的数据，从不跨越 Command 边界——
//! 暴露给前端的只有来源摘要（`KbSourceSummary`）与问答结果（`KbAnswerSource`）。

use serde::{Deserialize, Serialize};

/// 一个知识库来源目录（`kb_sources` 表一行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbSource {
    pub id: String,
    pub path: String,
    pub created_at: i64,
}

/// 单个来源目录的摄入状态摘要——`kb_list_sources` 返回这个而不是裸 `KbSource`，
/// 前端据此显示"已索引 N / 共 M，失败 K"，不需要额外一个 Command 才能拿到进度。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbSourceSummary {
    pub id: String,
    pub path: String,
    pub created_at: i64,
    pub doc_count: u32,
    pub indexed_count: u32,
    pub failed_count: u32,
}

/// 文档摄入状态：`pending` 待摄入（新增/内容变化，还没跑嵌入）、`indexed` 已摄入
/// 成功、`failed` 本次尝试失败（同 D3/AI3"单个失败只跳过"先例，不阻塞其余文档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KbDocStatus {
    Pending,
    Indexed,
    Failed,
}

impl KbDocStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexed => "indexed",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for KbDocStatus {
    type Err = crate::Aa4cError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "indexed" => Ok(Self::Indexed),
            "failed" => Ok(Self::Failed),
            other => Err(crate::Aa4cError::Protocol(format!(
                "unknown kb doc status: {other}"
            ))),
        }
    }
}

/// 一条被扫描到的文档（`kb_documents` 表一行）；`hash`/`mtime` 用于增量摄入判断
/// 内容是否真的变化了（同 `sync_index` 既有的增量扫描惯例）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbDocument {
    pub id: String,
    pub source_id: String,
    pub rel_path: String,
    pub mtime: i64,
    pub hash: String,
    pub status: KbDocStatus,
    pub updated_at: i64,
}

/// 问答回答里的一条引用来源——只给前端"这个文件路径贡献过内容"这一件事，
/// 不暴露具体命中的是哪个 chunk/相似度分数（§6：LLM 输出只呈现给人看，
/// 引用列表本身也只是辅助信息，不是需要精确溯源的审计日志）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbAnswerSource {
    pub path: String,
}
