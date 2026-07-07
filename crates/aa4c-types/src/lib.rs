//! AA4C 公共类型：设备、任务、事件、错误。
//!
//! 本 crate 被所有其他 crate 依赖，禁止引入任何 I/O 依赖。
//! 类型契约见 API_DESIGN.md §3，协议常量见 PROTOCOL.md §0。

#![forbid(unsafe_code)]

mod device;
mod error;
mod event;
mod server_addr;
mod settings;
mod sync;
mod transfer;

pub use device::{DeviceId, DeviceInfo, Platform, TrustLevel};
pub use error::{Aa4cError, Result};
pub use event::{ConnectionVia, CoreEvent};
pub use server_addr::ServerAddr;
pub use settings::Settings;
pub use sync::{
    RemoteIndexEntry, ScopeKind, SyncConflict, SyncFileEntry, SyncScope, SyncStatus, UnifiedFile,
};
pub use transfer::{Direction, FileStatus, TaskId, TransferFile, TransferStatus, TransferTask};

/// 协议版本（PROTOCOL.md §0）。V0.3 起为 3：新增广域网 QUIC 会话层 + 断点续传
/// （`ResumeReport`，PROTOCOL.md §13）。握手 `min(双方)` 协商；与更旧对端相遇时
/// 自动降级为对方版本的行为（见 §14）。
pub const PROTO_VERSION: u16 = 3;

/// 支持跨设备索引交换 / 按需拉取所需的最低协商版本（PROTOCOL.md §8b / §14）。
/// 握手谈成的 proto < 此值即对端为 v1，跳过一切同步消息（优雅降级，不发 v2 帧）。
pub const SYNC_PROTO_VERSION: u16 = 2;

/// 支持断点续传所需的最低协商版本（PROTOCOL.md §13 / CONNECT_DESIGN.md §5，里程碑 C1）。
/// 握手谈成的 proto ≥ 此值时，双方都确定性地交换 `ResumeReport`（不是尝试性的）：
/// proto < 此值的一方根本不认识该消息，两端都不发送，行为与旧版完全一致。
pub const RESUME_PROTO_VERSION: u16 = 3;

/// 默认监听端口（PROTOCOL.md §0）。
pub const DEFAULT_PORT: u16 = 42420;

/// mDNS 服务类型（PROTOCOL.md §1）。
pub const SERVICE_TYPE: &str = "_aa4c._tcp.local.";

/// 分块大小：4 MiB（PROTOCOL.md §0）。
pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// 最大帧长：16 MiB（PROTOCOL.md §3）。
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

// 协议不变量：分块必须能放进一帧
const _: () = assert!(MAX_FRAME_LEN > CHUNK_SIZE);
