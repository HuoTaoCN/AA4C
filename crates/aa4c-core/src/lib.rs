//! AA4C Core：应用生命周期、事件总线、服务编排。
//!
//! Core 只协调，不实现业务（AGENTS.md Core 规则）。
//! 接口契约见 API_DESIGN.md §8，将在 M6 里程碑实现。

#![forbid(unsafe_code)]

/// AA4C 版本号（与 workspace 版本一致）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
