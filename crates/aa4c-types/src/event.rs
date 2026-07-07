//! 核心事件（API_DESIGN.md §3.2）。事件总线与 Tauri 事件共用。

use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceInfo, TaskId, TransferTask};

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
}
