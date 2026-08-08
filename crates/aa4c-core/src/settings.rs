//! 设置读写：把 [`Settings`] 聚合视图映射到 settings 表的若干 KV
//! （值 JSON 编码，DATABASE_SCHEMA.md §2.4），缺失项用平台默认值补齐。

use std::path::PathBuf;

use aa4c_store::Store;
use aa4c_types::{Aa4cError, Platform, Result, Settings, DEFAULT_PORT};

pub(crate) const KEY_DEVICE_NAME: &str = "device_name";
pub(crate) const KEY_SAVE_DIR: &str = "save_dir";
pub(crate) const KEY_AUTO_ACCEPT: &str = "auto_accept_from_trusted";
pub(crate) const KEY_LISTEN_PORT: &str = "listen_port";
pub(crate) const KEY_SERVER_URL: &str = "server_url";
pub(crate) const KEY_ENABLE_REMOTE: &str = "enable_remote";
pub(crate) const KEY_ENABLE_PORT_MAPPING: &str = "enable_port_mapping";
pub(crate) const KEY_DOWNLOAD_DIR: &str = "download_dir";
pub(crate) const KEY_DOWNLOAD_SPEED_LIMIT_KBPS: &str = "download_speed_limit_kbps";
pub(crate) const KEY_DOWNLOAD_CONCURRENCY: &str = "download_concurrency";
pub(crate) const KEY_DOWNLOAD_MAX_CONNECTIONS_PER_FILE: &str = "download_max_connections_per_file";
pub(crate) const KEY_DOWNLOAD_UPLOAD_LIMIT_KBPS: &str = "download_upload_limit_kbps";
pub(crate) const KEY_DOWNLOAD_USER_AGENT: &str = "download_user_agent";
pub(crate) const KEY_DOWNLOAD_PROXY: &str = "download_proxy";
pub(crate) const KEY_DOWNLOAD_PROXY_BYPASS: &str = "download_proxy_bypass";
pub(crate) const KEY_BT_TRACKERS: &str = "bt_trackers";
pub(crate) const KEY_DOWNLOAD_RESUME_ON_START: &str = "download_resume_on_start";
pub(crate) const KEY_BT_RATIO_LIMIT: &str = "bt_ratio_limit";
pub(crate) const KEY_BT_IDLE_SEEDING_LIMIT_MINUTES: &str = "bt_idle_seeding_limit_minutes";
pub(crate) const KEY_ARCHIVE_ROOT: &str = "archive_root";
pub(crate) const KEY_ARCHIVE_AUTO_ENABLED: &str = "archive_auto_enabled";
pub(crate) const KEY_AI_MODELS_DIR: &str = "ai_models_dir";
pub(crate) const KEY_AI_CHAT_MODEL: &str = "ai_chat_model";
pub(crate) const KEY_AI_EMBEDDING_MODEL: &str = "ai_embedding_model";
pub(crate) const KEY_AI_IDLE_TIMEOUT_MINUTES: &str = "ai_idle_timeout_minutes";

/// 默认空闲超时（分钟）：ARCHIVE_DESIGN.md §3.3。
pub(crate) const DEFAULT_AI_IDLE_TIMEOUT_MINUTES: u32 = 10;

/// 平台默认接收目录：`~/Downloads/AA4C`（取不到下载目录时退回临时目录）。
pub(crate) fn default_save_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("AA4C")
}

/// 平台默认下载目录：系统下载目录本身（`~/Downloads`），**不**像 `save_dir` 那样
/// 再拼一层 `AA4C` 子目录——必须落在 `save_dir` 子树之外，否则 Inbox 会把下载
/// 内容自动索引、分享给全部完全信任设备（DOWNLOAD_DESIGN.md §5/§7，里程碑 D1）。
pub(crate) fn default_download_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(std::env::temp_dir)
}

/// 平台默认归档根目录：系统文档目录下的 `AA4C归档`（ARCHIVE_DESIGN.md §2.5，
/// 里程碑 AI1）。同 `default_save_dir` 的取舍——落在文档目录而不是下载目录，
/// 避免和 `download_dir` 产生嵌套歧义。
pub(crate) fn default_archive_root() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("AA4C归档")
}

/// 默认模型目录：`<归档根>/模型`——与内置"模型"归档规则的目标目录故意同址
/// （ARCHIVE_DESIGN.md §3.5：下载 GGUF → 自动归档进模型目录 → 模型库立即可见）。
/// 依赖 `archive_root` 而不是独立算一个平台路径，所以是个函数不是常量。
pub(crate) fn default_ai_models_dir(archive_root: &str) -> PathBuf {
    PathBuf::from(archive_root).join("模型")
}

/// 本机默认设备名：取 hostname 并去掉 mDNS 风格的 `.local` 等后缀；
/// hostname 缺失或为无意义值（如 Android 的 `localhost`）时，回落到平台名。
pub(crate) fn default_device_name() -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    // "Huo-MacBook.local" → "Huo-MacBook"；Windows 主机名通常无点，保持不变
    let trimmed = raw.split('.').next().unwrap_or("").trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("localhost") {
        return platform_device_name();
    }
    trimmed.to_string()
}

/// 平台兜底名（hostname 不可用时）。
fn platform_device_name() -> String {
    match Platform::current() {
        Platform::Macos => "Mac",
        Platform::Windows => "Windows 电脑",
        Platform::Linux => "Linux 电脑",
        Platform::Android => "Android 手机",
        Platform::Ios => "iPhone",
        Platform::Server => "服务器",
    }
    .to_string()
}

/// 读取聚合设置，缺失项用默认值补齐。
///
/// `fallback_name` / `fallback_save_dir` 为平台注入的缺省值（设备名取 hostname、
/// 保存目录由 Tauri path resolver 提供，见 API_DESIGN §11）。
pub(crate) async fn load(
    store: &Store,
    fallback_name: &str,
    fallback_save_dir: &str,
) -> Result<Settings> {
    // `ai_models_dir` 的默认值依赖 `archive_root`，提前算出来，struct 字面量里
    // 才能直接引用（Rust struct 字面量按写出的顺序求值，但不能跨字段互相借用
    // "正在构造中"的另一个字段）。
    let archive_root = get_json(store, KEY_ARCHIVE_ROOT)
        .await?
        .unwrap_or_else(|| default_archive_root().to_string_lossy().into_owned());

    Ok(Settings {
        device_name: get_json(store, KEY_DEVICE_NAME)
            .await?
            .unwrap_or_else(|| fallback_name.to_string()),
        save_dir: get_json(store, KEY_SAVE_DIR)
            .await?
            .unwrap_or_else(|| fallback_save_dir.to_string()),
        auto_accept_from_trusted: get_json(store, KEY_AUTO_ACCEPT).await?.unwrap_or(false),
        listen_port: get_json(store, KEY_LISTEN_PORT)
            .await?
            .unwrap_or(DEFAULT_PORT),
        server_url: get_json(store, KEY_SERVER_URL).await?,
        enable_remote: get_json(store, KEY_ENABLE_REMOTE).await?.unwrap_or(false),
        // 默认 true：见 `Settings::enable_port_mapping` 的文档——外层 `enable_remote`
        // 默认关闭已经保证了「不打开就不出网」，这一项是在那之内的取舍。
        enable_port_mapping: get_json(store, KEY_ENABLE_PORT_MAPPING)
            .await?
            .unwrap_or(true),
        download_dir: get_json(store, KEY_DOWNLOAD_DIR)
            .await?
            .unwrap_or_else(|| default_download_dir().to_string_lossy().into_owned()),
        download_speed_limit_kbps: get_json(store, KEY_DOWNLOAD_SPEED_LIMIT_KBPS).await?,
        download_concurrency: get_json(store, KEY_DOWNLOAD_CONCURRENCY).await?,
        download_max_connections_per_file: get_json(store, KEY_DOWNLOAD_MAX_CONNECTIONS_PER_FILE)
            .await?,
        download_upload_limit_kbps: get_json(store, KEY_DOWNLOAD_UPLOAD_LIMIT_KBPS).await?,
        download_user_agent: get_json(store, KEY_DOWNLOAD_USER_AGENT).await?,
        download_proxy: get_json(store, KEY_DOWNLOAD_PROXY).await?,
        download_proxy_bypass: get_json(store, KEY_DOWNLOAD_PROXY_BYPASS).await?,
        bt_trackers: get_json(store, KEY_BT_TRACKERS).await?,
        download_resume_on_start: get_json(store, KEY_DOWNLOAD_RESUME_ON_START)
            .await?
            .unwrap_or(false),
        bt_ratio_limit: get_json(store, KEY_BT_RATIO_LIMIT).await?,
        bt_idle_seeding_limit_minutes: get_json(store, KEY_BT_IDLE_SEEDING_LIMIT_MINUTES).await?,
        ai_models_dir: get_json(store, KEY_AI_MODELS_DIR)
            .await?
            .unwrap_or_else(|| {
                default_ai_models_dir(&archive_root)
                    .to_string_lossy()
                    .into_owned()
            }),
        archive_auto_enabled: get_json(store, KEY_ARCHIVE_AUTO_ENABLED)
            .await?
            .unwrap_or(true),
        ai_chat_model: get_json(store, KEY_AI_CHAT_MODEL).await?,
        ai_embedding_model: get_json(store, KEY_AI_EMBEDDING_MODEL).await?,
        ai_idle_timeout_minutes: get_json(store, KEY_AI_IDLE_TIMEOUT_MINUTES)
            .await?
            .unwrap_or(DEFAULT_AI_IDLE_TIMEOUT_MINUTES),
        archive_root,
    })
}

/// 持久化聚合设置（逐键写入）。
pub(crate) async fn save(store: &Store, s: &Settings) -> Result<()> {
    set_json(store, KEY_DEVICE_NAME, &s.device_name).await?;
    set_json(store, KEY_SAVE_DIR, &s.save_dir).await?;
    set_json(store, KEY_AUTO_ACCEPT, &s.auto_accept_from_trusted).await?;
    set_json(store, KEY_LISTEN_PORT, &s.listen_port).await?;
    set_json(store, KEY_SERVER_URL, &s.server_url).await?;
    set_json(store, KEY_ENABLE_REMOTE, &s.enable_remote).await?;
    set_json(store, KEY_ENABLE_PORT_MAPPING, &s.enable_port_mapping).await?;
    set_json(store, KEY_DOWNLOAD_DIR, &s.download_dir).await?;
    set_json(
        store,
        KEY_DOWNLOAD_SPEED_LIMIT_KBPS,
        &s.download_speed_limit_kbps,
    )
    .await?;
    set_json(store, KEY_DOWNLOAD_CONCURRENCY, &s.download_concurrency).await?;
    set_json(
        store,
        KEY_DOWNLOAD_MAX_CONNECTIONS_PER_FILE,
        &s.download_max_connections_per_file,
    )
    .await?;
    set_json(
        store,
        KEY_DOWNLOAD_UPLOAD_LIMIT_KBPS,
        &s.download_upload_limit_kbps,
    )
    .await?;
    set_json(store, KEY_DOWNLOAD_USER_AGENT, &s.download_user_agent).await?;
    set_json(store, KEY_DOWNLOAD_PROXY, &s.download_proxy).await?;
    set_json(store, KEY_DOWNLOAD_PROXY_BYPASS, &s.download_proxy_bypass).await?;
    set_json(store, KEY_BT_TRACKERS, &s.bt_trackers).await?;
    set_json(
        store,
        KEY_DOWNLOAD_RESUME_ON_START,
        &s.download_resume_on_start,
    )
    .await?;
    set_json(store, KEY_BT_RATIO_LIMIT, &s.bt_ratio_limit).await?;
    set_json(
        store,
        KEY_BT_IDLE_SEEDING_LIMIT_MINUTES,
        &s.bt_idle_seeding_limit_minutes,
    )
    .await?;
    set_json(store, KEY_ARCHIVE_ROOT, &s.archive_root).await?;
    set_json(store, KEY_ARCHIVE_AUTO_ENABLED, &s.archive_auto_enabled).await?;
    set_json(store, KEY_AI_MODELS_DIR, &s.ai_models_dir).await?;
    set_json(store, KEY_AI_CHAT_MODEL, &s.ai_chat_model).await?;
    set_json(store, KEY_AI_EMBEDDING_MODEL, &s.ai_embedding_model).await?;
    set_json(
        store,
        KEY_AI_IDLE_TIMEOUT_MINUTES,
        &s.ai_idle_timeout_minutes,
    )
    .await?;
    Ok(())
}

async fn get_json<T: serde::de::DeserializeOwned>(store: &Store, key: &str) -> Result<Option<T>> {
    match store.get_setting(key).await? {
        // 非法/旧值静默退回默认，避免单个坏键阻断启动
        Some(raw) => Ok(serde_json::from_str(&raw).ok()),
        None => Ok(None),
    }
}

async fn set_json<T: serde::Serialize>(store: &Store, key: &str, value: &T) -> Result<()> {
    let raw = serde_json::to_string(value)
        .map_err(|e| Aa4cError::Protocol(format!("settings encode failed: {e}")))?;
    store.set_setting(key, &raw).await
}
