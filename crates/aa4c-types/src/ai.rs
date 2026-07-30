//! AI 模型库类型（V0.5 里程碑 AI2.4，ARCHIVE_DESIGN.md §3.5）。

use serde::{Deserialize, Serialize};

use crate::ModelMeta;

/// `ai_models_dir` 下扫描到的一个本地 GGUF 模型（`list_local_models` 的元素）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    /// 绝对路径——选定对话/嵌入模型时把这个值写回 `Settings.ai_chat_model`/
    /// `ai_embedding_model`。
    pub path: String,
    pub meta: ModelMeta,
}

/// 一个槽位（对话/嵌入）的静态快照——`configured` 是"有没有配模型文件"，
/// `running` 是"进程现在是不是真的活着"，两者独立（配了模型但懒启动还没
/// 触发时 `configured=true, running=false` 是正常状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSlotStatus {
    pub configured: bool,
    pub running: bool,
}

/// `get_ai_status` 的返回：两个槽位各自的快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub chat: AiSlotStatus,
    pub embedding: AiSlotStatus,
}
