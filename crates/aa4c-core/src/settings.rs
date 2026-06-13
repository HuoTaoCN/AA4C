//! 设置读写：把 [`Settings`] 聚合视图映射到 settings 表的若干 KV
//! （值 JSON 编码，DATABASE_SCHEMA.md §2.4），缺失项用平台默认值补齐。

use std::path::PathBuf;

use aa4c_store::Store;
use aa4c_types::{Aa4cError, Result, Settings, DEFAULT_PORT};

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

/// 本机默认设备名：操作系统 hostname；取不到时给一个通用名。
pub(crate) fn default_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AA4C 设备".to_string())
}

/// 读取聚合设置，缺失项用默认值补齐。`fallback_name` 用于 device_name 缺省。
pub(crate) async fn load(store: &Store, fallback_name: &str) -> Result<Settings> {
    Ok(Settings {
        device_name: get_json(store, KEY_DEVICE_NAME)
            .await?
            .unwrap_or_else(|| fallback_name.to_string()),
        save_dir: get_json(store, KEY_SAVE_DIR)
            .await?
            .unwrap_or_else(|| default_save_dir().to_string_lossy().into_owned()),
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
