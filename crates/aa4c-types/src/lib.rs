//! AA4C 公共类型：设备、任务、事件、错误。
//!
//! 本 crate 被所有其他 crate 依赖，禁止引入任何 I/O 依赖。
//! 类型契约见 API_DESIGN.md §3，协议常量见 PROTOCOL.md §0。

#![forbid(unsafe_code)]

mod ai;
mod archive;
mod device;
mod download;
mod error;
mod event;
mod kb;
mod server_addr;
mod settings;
mod share;
mod sync;
mod transfer;

pub use ai::{AiSlotStatus, AiStatus, LocalModel, Suggestion};
pub use archive::{
    ArchiveAction, ArchiveCategory, ArchiveEntry, ArchiveLogEntry, ArchiveMatch, ArchiveRule,
    ArchiveTag, ModelMeta, TagSource,
};
pub use device::{DeviceId, DeviceInfo, Platform, TrustLevel};
pub use download::{DownloadKind, DownloadStatus, DownloadTask};
pub use error::{Aa4cError, Result};
pub use event::{AiEngineStatus, AiSlot, ConnectionVia, CoreEvent};
pub use kb::{KbAnswerSource, KbDocStatus, KbDocument, KbSource, KbSourceSummary};
pub use server_addr::ServerAddr;
pub use settings::Settings;
pub use share::{Share, ShareAccess, ShareLink};
pub use sync::{
    RemoteIndexEntry, ScopeKind, SyncConflict, SyncFileEntry, SyncScope, SyncStatus, UnifiedFile,
};
pub use transfer::{Direction, FileStatus, TaskId, TransferFile, TransferStatus, TransferTask};

/// 协议版本（PROTOCOL.md §0）。V0.3 起为 4：新增广域网 QUIC 会话层 + 断点续传
/// （`ResumeReport`，PROTOCOL.md §13，proto=3）+ 分享链接的 `ShareRequest`
/// （PROTOCOL.md §16，proto=4，里程碑 C6）。proto=5：配对时交换 `server_hint`
/// （`PairServerHint`，PROTOCOL.md §17）。握手 `min(双方)` 协商；与更旧对端相遇时
/// 自动降级为对方版本的行为（见 §14）。
pub const PROTO_VERSION: u16 = 5;

/// 支持跨设备索引交换 / 按需拉取所需的最低协商版本（PROTOCOL.md §8b / §14）。
/// 握手谈成的 proto < 此值即对端为 v1，跳过一切同步消息（优雅降级，不发 v2 帧）。
pub const SYNC_PROTO_VERSION: u16 = 2;

/// 支持断点续传所需的最低协商版本（PROTOCOL.md §13 / CONNECT_DESIGN.md §5，里程碑 C1）。
/// 握手谈成的 proto ≥ 此值时，双方都确定性地交换 `ResumeReport`（不是尝试性的）：
/// proto < 此值的一方根本不认识该消息，两端都不发送，行为与旧版完全一致。
pub const RESUME_PROTO_VERSION: u16 = 3;

/// 支持分享链接（`ShareRequest`）所需的最低协商版本（PROTOCOL.md §16，CONNECT_DESIGN.md
/// §7，里程碑 C6）。打开分享链接时若对端协商 proto 低于此值，直接报错不发送——分享方
/// 版本太旧，根本不认识这个消息（优雅降级，同 `SYNC_PROTO_VERSION` 的处理方式）。
pub const SHARE_PROTO_VERSION: u16 = 4;

/// 支持配对时交换 `server_hint`（`PairServerHint`）所需的最低协商版本（PROTOCOL.md §17，
/// V0.3 遗留 gap 补完）。proto ≥ 此值时配对双方**确定性地**互相声明各自当前配置的 home
/// server 地址（`aa4c_store::DeviceRecord::server_hint`），用于后续跨服务器好友寻址；
/// proto < 此值的一方不认识 `PairServerHint`，两端都不发送，行为与旧版完全一致（同
/// `RESUME_PROTO_VERSION` 的 gate 惯例）。
pub const SERVER_HINT_PROTO_VERSION: u16 = 5;

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
