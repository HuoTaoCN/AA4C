//! 传输任务类型（API_DESIGN.md §3）。

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Aa4cError, DeviceId, Result};

/// 任务 ID = UUID v4 字符串。
pub type TaskId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Send,
    Recv,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Recv => "recv",
        }
    }
}

impl FromStr for Direction {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "send" => Ok(Self::Send),
            "recv" => Ok(Self::Recv),
            other => Err(Aa4cError::Protocol(format!("invalid direction: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    /// 等待接收方确认。
    WaitingAccept,
    Transferring,
    /// 发送方主动暂停。实现上就是**静默断开连接**——不发 `Cancel`，接收端因此
    /// 保留已落盘的 `.aa4c-part`（PROTOCOL.md §7 规则 3：只有明确取消才清理），
    /// 「继续」时用同一个 task_id 重新发起，走既有的断点续传协商（§13）接上。
    /// 非终态：`is_terminal()` 为 false。
    Paused,
    Done,
    Failed,
    Cancelled,
    Rejected,
}

impl TransferStatus {
    /// 数据库 CHECK 约束中使用的稳定字符串（DATABASE_SCHEMA.md §2.2）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WaitingAccept => "waiting_accept",
            Self::Transferring => "transferring",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }

    /// 任务是否已进入终态。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Cancelled | Self::Rejected
        )
    }
}

impl FromStr for TransferStatus {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "waiting_accept" => Ok(Self::WaitingAccept),
            "transferring" => Ok(Self::Transferring),
            "paused" => Ok(Self::Paused),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "rejected" => Ok(Self::Rejected),
            other => Err(Aa4cError::Protocol(format!(
                "invalid transfer status: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Pending,
    Transferring,
    Done,
    Failed,
}

impl FileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Transferring => "transferring",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for FileStatus {
    type Err = Aa4cError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "transferring" => Ok(Self::Transferring),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(Aa4cError::Protocol(format!("invalid file status: {other}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferFile {
    /// 相对路径（'/' 分隔，文件夹传输时保留层级）。
    pub rel_path: String,
    pub size: u64,
    /// BLAKE3 hex，传输完成后填充。
    pub hash: Option<String>,
    pub status: FileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: TaskId,
    pub direction: Direction,
    pub peer: DeviceId,
    pub files: Vec<TransferFile>,
    pub status: TransferStatus,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    /// unix 毫秒。
    pub created_at: i64,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_str_roundtrip() {
        for s in [
            TransferStatus::WaitingAccept,
            TransferStatus::Transferring,
            TransferStatus::Paused,
            TransferStatus::Done,
            TransferStatus::Failed,
            TransferStatus::Cancelled,
            TransferStatus::Rejected,
        ] {
            assert_eq!(s.as_str().parse::<TransferStatus>().unwrap(), s);
        }
        assert!(TransferStatus::Done.is_terminal());
        assert!(!TransferStatus::Transferring.is_terminal());
        assert!(!TransferStatus::Paused.is_terminal());
    }

    #[test]
    fn task_json_roundtrip_is_camel_case() {
        let task = TransferTask {
            id: "11111111-2222-3333-4444-555555555555".into(),
            direction: Direction::Send,
            peer: "ab".repeat(32),
            files: vec![TransferFile {
                rel_path: "照片/IMG_2024.jpg".into(),
                size: 1024,
                hash: None,
                status: FileStatus::Pending,
            }],
            status: TransferStatus::WaitingAccept,
            total_bytes: 1024,
            transferred_bytes: 0,
            created_at: 1_750_000_000_000,
            error: None,
        };
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["status"], "waiting_accept");
        assert_eq!(json["totalBytes"], 1024);
        assert_eq!(json["files"][0]["relPath"], "照片/IMG_2024.jpg");
        let back: TransferTask = serde_json::from_value(json).unwrap();
        assert_eq!(back, task);
    }
}
