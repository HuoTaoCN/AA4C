//! 统一错误类型（API_DESIGN.md §3.1）。

use crate::DeviceId;

#[derive(Debug, thiserror::Error)]
pub enum Aa4cError {
    #[error("device not found: {0}")]
    DeviceNotFound(DeviceId),
    #[error("device not paired: {0}")]
    NotPaired(DeviceId),
    #[error("pairing rejected")]
    PairingRejected,
    #[error("pairing pin mismatch")]
    PinMismatch,
    #[error("transfer rejected by peer")]
    TransferRejected,
    #[error("hash mismatch for {path}")]
    HashMismatch { path: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
    #[error("capability unavailable: {0}")]
    Unavailable(String),
}

impl Aa4cError {
    /// 稳定的错误码（蛇形命名），用于 Tauri 层的 `{ code, message }` 映射
    /// 与 UI 文案表（UI_DESIGN_SPEC.md §6）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound(_) => "device_not_found",
            Self::NotPaired(_) => "not_paired",
            Self::PairingRejected => "pairing_rejected",
            Self::PinMismatch => "pin_mismatch",
            Self::TransferRejected => "transfer_rejected",
            Self::HashMismatch { .. } => "hash_mismatch",
            Self::Io(_) => "io",
            Self::Db(_) => "db",
            Self::Network(_) => "network",
            Self::Protocol(_) => "protocol",
            Self::Cancelled => "cancelled",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

pub type Result<T> = std::result::Result<T, Aa4cError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_snake_case() {
        let err = Aa4cError::NotPaired("abc".into());
        assert_eq!(err.code(), "not_paired");
        assert_eq!(err.to_string(), "device not paired: abc");
    }
}
