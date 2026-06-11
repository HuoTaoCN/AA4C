//! AA4C 公共类型：设备、任务、事件、错误。
//!
//! 本 crate 被所有其他 crate 依赖，禁止引入任何 I/O 依赖。
//! 完整类型定义见 API_DESIGN.md §3，将在 M1 里程碑实现。

#![forbid(unsafe_code)]

/// 协议版本（PROTOCOL.md §0）。
pub const PROTO_VERSION: u16 = 1;

/// 默认监听端口（PROTOCOL.md §0）。
pub const DEFAULT_PORT: u16 = 42420;

/// mDNS 服务类型（PROTOCOL.md §1）。
pub const SERVICE_TYPE: &str = "_aa4c._tcp.local.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_stable() {
        assert_eq!(PROTO_VERSION, 1);
        assert_eq!(DEFAULT_PORT, 42420);
        assert!(SERVICE_TYPE.ends_with("._tcp.local."));
    }
}
