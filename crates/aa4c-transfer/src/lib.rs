//! AA4C 传输引擎：ATP 帧编解码、文件收发、BLAKE3 校验。
//!
//! 协议规范见 PROTOCOL.md，接口契约见 API_DESIGN.md §6，将在 M5 里程碑实现。

#![forbid(unsafe_code)]
