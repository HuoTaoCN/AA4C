//! 设置读写：把 [`Settings`] 聚合视图映射到 settings 表的若干 KV
//! （值 JSON 编码，DATABASE_SCHEMA.md §2.4），缺失项用平台默认值补齐。

use std::path::PathBuf;

use aa4c_store::Store;
use aa4c_types::{Aa4cError, Platform, Result, Settings, DEFAULT_PORT};

pub(crate) const KEY_DEVICE_NAME: &str = "device_name";
pub(crate) const KEY_SAVE_DIR: &str = "save_dir";
pub(crate) const KEY_AUTO_ACCEPT: &str = "auto_accept_from_trusted";
pub(crate) const KEY_LISTEN_PORT: &str = "listen_port";

/// 平台默认接收目录：`~/Downloads/AA4C`（取不到下载目录时退回临时目录）。
pub(crate) fn default_save_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("AA4C")
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
    })
}

/// 持久化聚合设置（逐键写入）。
pub(crate) async fn save(store: &Store, s: &Settings) -> Result<()> {
    set_json(store, KEY_DEVICE_NAME, &s.device_name).await?;
    set_json(store, KEY_SAVE_DIR, &s.save_dir).await?;
    set_json(store, KEY_AUTO_ACCEPT, &s.auto_accept_from_trusted).await?;
    set_json(store, KEY_LISTEN_PORT, &s.listen_port).await?;
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
