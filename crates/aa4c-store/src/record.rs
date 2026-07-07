//! 数据库记录类型（DATABASE_SCHEMA.md §2.1）。

use aa4c_types::{DeviceId, Platform, TrustLevel};

/// devices 表的一行。
///
/// `created_at` / `updated_at` 由 Store 在写入时维护：
/// 调用方传入的值会被忽略，插入时两者取当前时间，更新时仅刷新 `updated_at`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: DeviceId,
    pub name: String,
    pub platform: Platform,
    /// Ed25519 公钥（32 字节）。
    pub public_key: Vec<u8>,
    pub trusted: bool,
    /// 信任分级（配对默认 `Friend`，可升级为 `Full`）。
    pub trust_level: TrustLevel,
    /// 配对完成时间（unix 毫秒）。
    pub paired_at: Option<i64>,
    /// 最近一次在线时间（unix 毫秒）。
    pub last_seen_at: Option<i64>,
    /// 最近一次发现的地址 "ip:port"。
    pub last_addr: Option<String>,
    /// 对端 home server 地址（`aa4c://host:port#fp`），可空（CONNECT_DESIGN.md §3.4，
    /// 里程碑 C2）。本里程碑只落库/查询，线路层交换留待后续里程碑。
    pub server_hint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
