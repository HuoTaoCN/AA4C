//! 核心事件（API_DESIGN.md §3.2）。事件总线与 Tauri 事件共用。

use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceInfo, KbAnswerSource, TaskId, TransferTask};

/// 一次连接实际走的档位（CONNECT_DESIGN.md §2 连接阶梯，里程碑 C4 连接质量 + C5 打洞）。
/// 局域网直连与公网直连对上层而言无区别，合并为 `Direct`；`Punch` 是打洞成功后升级
/// 成的 QUIC 直连（里程碑 C5）——虽然最终也是「直连」，但单独报出来是因为它经历了
/// 候选交换这一步，值得让 UI 区分「一上来就直连」和「打洞打出来的直连」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionVia {
    Direct,
    Punch,
    Relay,
}

/// AI 引擎的两个独立槽位（对话/嵌入，ARCHIVE_DESIGN.md §3.3）——各自独立
/// 进程、独立模型、独立生命周期，一个槽位的状态变化不影响另一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiSlot {
    Chat,
    Embedding,
}

/// AI 引擎槽位的生命周期状态（ARCHIVE_DESIGN.md §3.3：懒启动 + 空闲自停）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEngineStatus {
    /// 正在拉起进程 / 等待模型加载完成健康检查通过。
    Starting,
    /// 健康检查通过，可以接受推理请求。
    Ready,
    /// 空闲超时后已优雅退出（正常降级，不是错误）。
    Stopped,
    /// 模型未配置 / 启动失败 / 健康检查超时——同下载能力缺失的既有
    /// `Aa4cError::Unavailable` 语义。
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CoreEvent {
    DeviceFound(DeviceInfo),
    #[serde(rename_all = "camelCase")]
    DeviceLost {
        id: DeviceId,
    },
    DeviceUpdated(DeviceInfo),

    /// 对方请求与本机配对。
    #[serde(rename_all = "camelCase")]
    PairingRequest {
        session_id: String,
        peer: DeviceInfo,
    },
    /// 双方界面需要展示的 6 位确认码。
    #[serde(rename_all = "camelCase")]
    PairingPin {
        session_id: String,
        pin: String,
    },
    #[serde(rename_all = "camelCase")]
    PairingResult {
        session_id: String,
        peer: DeviceId,
        success: bool,
    },

    /// 对方请求向本机发送文件。
    #[serde(rename_all = "camelCase")]
    TransferRequest {
        task: TransferTask,
    },
    /// 出站连接已建立、即将开始传输（里程碑 C4 连接质量，CONNECT_DESIGN.md §2/§12）：
    /// 只在**发起方**（send/fetch）触发一次，告诉 UI 这次实际走的是哪一档，仅存于当次
    /// 会话内存（不落库，历史记录不含这个字段——见 HANDOFF.md 的取舍）。
    #[serde(rename_all = "camelCase")]
    TransferConnected {
        task_id: TaskId,
        via: ConnectionVia,
    },
    #[serde(rename_all = "camelCase")]
    TransferProgress {
        task_id: TaskId,
        transferred_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
        current_file: String,
    },
    #[serde(rename_all = "camelCase")]
    TransferDone {
        task_id: TaskId,
    },
    #[serde(rename_all = "camelCase")]
    TransferFailed {
        task_id: TaskId,
        error: String,
    },

    /// 本机同步索引发生变化（扫描完成），UI 应重新拉取统一文件视图。
    SyncIndexUpdated,

    /// 下载进度（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md §5）：状态迁移必发，进行中按数秒级
    /// 节流（不落库，前端本地维护——同 `TransferProgress` 的既有先例）。
    /// `seeders`/`peers`/`ratio` 是 D2（Transmission/BT）专属字段，HTTP 任务恒为
    /// `None` 且不出现在 JSON 里（`skip_serializing_if`）——同 `save_path` 不落库、
    /// 只进事件的既有先例（DOWNLOAD_DESIGN.md §3.6.4：BT 专属信息只进事件不落库），
    /// 复用同一个事件变体而不是另开一个 `BtProgress`，让 D1+D2 任务在前端走同一条
    /// 进度处理路径（对应 D3「统一任务中心」的目标）。
    #[serde(rename_all = "camelCase")]
    DownloadProgress {
        task_id: TaskId,
        downloaded_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        seeders: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        peers: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        ratio: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    DownloadDone {
        task_id: TaskId,
        save_path: String,
    },
    #[serde(rename_all = "camelCase")]
    DownloadFailed {
        task_id: TaskId,
        error: String,
    },

    /// 一次归档移动生效（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md §2.4）：自动（下载完成钩子）
    /// 或手动归档都会发这条，UI 据此刷新归档列表；`rule_id` 为 `None` 代表手动归档。
    #[serde(rename_all = "camelCase")]
    ArchiveApplied {
        entry_id: String,
        from_path: String,
        to_path: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        rule_id: Option<String>,
    },

    /// AI 引擎槽位状态变化（V0.5 里程碑 AI2，ARCHIVE_DESIGN.md §3.3）：懒启动/
    /// 空闲自停都经这条通知 UI（"正在加载模型…"/引导去模型库）。
    #[serde(rename_all = "camelCase")]
    AiEngineState {
        slot: AiSlot,
        status: AiEngineStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<String>,
    },

    /// AI 标签/分类建议批量队列进度（V0.5 里程碑 AI3，ARCHIVE_DESIGN.md §5）：
    /// 单并发逐个调用，每处理完一个文件发一次，`done == total` 即批量结束
    /// （不单独发"批量完成"事件，前端自己比较两个数字）。
    #[serde(rename_all = "camelCase")]
    AiSuggestProgress {
        done: u32,
        total: u32,
    },

    /// 知识库摄入进度（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）：单个来源目录内
    /// 逐文档嵌入，每处理完一个文档发一次，语义同 `AiSuggestProgress`
    /// （`done == total` 即这个来源摄入完成，不单独发"完成"事件）。
    #[serde(rename_all = "camelCase")]
    KbIngestProgress {
        source_id: String,
        done: u32,
        total: u32,
    },

    /// 知识库问答流式增量（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）：对话槽位
    /// SSE 转发，`request_id` 供前端关联到发起的那次提问（同一时刻只支持一个
    /// 进行中的问答，`request_id` 仍然带上是为了让前端能安全丢弃过期请求的
    /// 迟到增量，不是为了支持真正的并发问答）。
    #[serde(rename_all = "camelCase")]
    KbAnswerDelta {
        request_id: String,
        delta: String,
    },

    /// 知识库问答结束：附带引用来源列表（去重后的文件路径）。引擎失败/超时
    /// 时 `error` 非空，`sources` 为空数组。
    #[serde(rename_all = "camelCase")]
    KbAnswerDone {
        request_id: String,
        sources: Vec<KbAnswerSource>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<String>,
    },
}

impl CoreEvent {
    /// Tauri 事件名后缀（API_DESIGN.md §9.2：`aa4c://` + 蛇形事件名）。
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::DeviceFound(_) => "device_found",
            Self::DeviceLost { .. } => "device_lost",
            Self::DeviceUpdated(_) => "device_updated",
            Self::PairingRequest { .. } => "pairing_request",
            Self::PairingPin { .. } => "pairing_pin",
            Self::PairingResult { .. } => "pairing_result",
            Self::TransferRequest { .. } => "transfer_request",
            Self::TransferConnected { .. } => "transfer_connected",
            Self::TransferProgress { .. } => "transfer_progress",
            Self::TransferDone { .. } => "transfer_done",
            Self::TransferFailed { .. } => "transfer_failed",
            Self::SyncIndexUpdated => "sync_index_updated",
            Self::DownloadProgress { .. } => "download_progress",
            Self::DownloadDone { .. } => "download_done",
            Self::DownloadFailed { .. } => "download_failed",
            Self::ArchiveApplied { .. } => "archive_applied",
            Self::AiEngineState { .. } => "ai_engine_state",
            Self::AiSuggestProgress { .. } => "ai_suggest_progress",
            Self::KbIngestProgress { .. } => "kb_ingest_progress",
            Self::KbAnswerDelta { .. } => "kb_answer_delta",
            Self::KbAnswerDone { .. } => "kb_answer_done",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_shape_matches_api_design() {
        let event = CoreEvent::TransferProgress {
            task_id: "t1".into(),
            transferred_bytes: 500,
            total_bytes: 1000,
            speed_bps: 42_000_000,
            current_file: "IMG_2024.jpg".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "transfer_progress");
        assert_eq!(json["data"]["taskId"], "t1");
        assert_eq!(json["data"]["transferredBytes"], 500);
        assert_eq!(json["data"]["speedBps"], 42_000_000);
        assert_eq!(event.event_name(), "transfer_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn event_name_matches_serde_tag() {
        let event = CoreEvent::DeviceLost { id: "d1".into() };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], event.event_name());
    }

    /// 里程碑 C4：连接质量事件的 JSON 形状（camelCase `taskId` + snake_case 的 via 取值），
    /// 前端 `ConnectionVia` 类型按这个约定做字符串字面量联合。
    #[test]
    fn transfer_connected_json_shape() {
        let event = CoreEvent::TransferConnected {
            task_id: "t1".into(),
            via: ConnectionVia::Relay,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "transfer_connected");
        assert_eq!(json["data"]["taskId"], "t1");
        assert_eq!(json["data"]["via"], "relay");
        assert_eq!(event.event_name(), "transfer_connected");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 D1：下载进度事件的 JSON 形状（camelCase，同 `TransferProgress` 的既有约定）。
    /// HTTP 任务（D1）没有做种数/peer 数/分享率，三个字段应该整体不出现在 JSON 里
    /// （不是出现成 `null`）——`skip_serializing_if` 就是为了这一点。
    #[test]
    fn download_progress_json_shape() {
        let event = CoreEvent::DownloadProgress {
            task_id: "gid1".into(),
            downloaded_bytes: 500,
            total_bytes: 1000,
            speed_bps: 1_000_000,
            seeders: None,
            peers: None,
            ratio: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "download_progress");
        assert_eq!(json["data"]["taskId"], "gid1");
        assert_eq!(json["data"]["downloadedBytes"], 500);
        assert!(json["data"].get("seeders").is_none());
        assert!(json["data"].get("peers").is_none());
        assert!(json["data"].get("ratio").is_none());
        assert_eq!(event.event_name(), "download_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// D2：BT 任务的做种数/peer 数/分享率只进事件、不落库（DOWNLOAD_DESIGN.md
    /// §3.6.4），复用同一个 `DownloadProgress` 变体而不是另开一个 BT 专属事件。
    #[test]
    fn download_progress_json_shape_with_bt_fields() {
        let event = CoreEvent::DownloadProgress {
            task_id: "infohash1".into(),
            downloaded_bytes: 500,
            total_bytes: 1000,
            speed_bps: 1_000_000,
            seeders: Some(12),
            peers: Some(3),
            ratio: Some(1.5),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["data"]["seeders"], 12);
        assert_eq!(json["data"]["peers"], 3);
        assert_eq!(json["data"]["ratio"], 1.5);

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI2：AI 引擎槽位状态事件的 JSON 形状，`error` 缺省时整体不出现
    /// 在 JSON 里（同 `rule_id`/BT 专属字段的既有先例）。
    #[test]
    fn ai_engine_state_json_shape() {
        let event = CoreEvent::AiEngineState {
            slot: AiSlot::Embedding,
            status: AiEngineStatus::Starting,
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "ai_engine_state");
        assert_eq!(json["data"]["slot"], "embedding");
        assert_eq!(json["data"]["status"], "starting");
        assert!(json["data"].get("error").is_none());
        assert_eq!(event.event_name(), "ai_engine_state");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI3：批量建议进度事件的 JSON 形状——两个裸数字，没有可省略字段。
    #[test]
    fn ai_suggest_progress_json_shape() {
        let event = CoreEvent::AiSuggestProgress { done: 2, total: 5 };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "ai_suggest_progress");
        assert_eq!(json["data"]["done"], 2);
        assert_eq!(json["data"]["total"], 5);
        assert_eq!(event.event_name(), "ai_suggest_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI4：知识库摄入进度事件的 JSON 形状。
    #[test]
    fn kb_ingest_progress_json_shape() {
        let event = CoreEvent::KbIngestProgress {
            source_id: "src-1".into(),
            done: 3,
            total: 10,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "kb_ingest_progress");
        assert_eq!(json["data"]["sourceId"], "src-1");
        assert_eq!(json["data"]["done"], 3);
        assert_eq!(json["data"]["total"], 10);
        assert_eq!(event.event_name(), "kb_ingest_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI4：问答完成事件——`error` 缺省不出现在 JSON 里，同其余
    /// `skip_serializing_if` 字段的既有约定。
    #[test]
    fn kb_answer_done_json_shape_omits_absent_error() {
        let event = CoreEvent::KbAnswerDone {
            request_id: "req-1".into(),
            sources: vec![KbAnswerSource {
                path: "/tmp/notes/a.md".into(),
            }],
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "kb_answer_done");
        assert_eq!(json["data"]["requestId"], "req-1");
        assert_eq!(json["data"]["sources"][0]["path"], "/tmp/notes/a.md");
        assert!(json["data"].get("error").is_none());
        assert_eq!(event.event_name(), "kb_answer_done");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
