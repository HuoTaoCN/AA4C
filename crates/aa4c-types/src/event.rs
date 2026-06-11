//! 核心事件（API_DESIGN.md §3.2）。事件总线与 Tauri 事件共用。

use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceInfo, TaskId, TransferTask};

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
            Self::TransferProgress { .. } => "transfer_progress",
            Self::TransferDone { .. } => "transfer_done",
            Self::TransferFailed { .. } => "transfer_failed",
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
}
