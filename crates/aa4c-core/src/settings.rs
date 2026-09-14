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
pub(crate) const KEY_ENABLE_LOCAL_SERVER: &str = "enable_local_server";
pub(crate) const KEY_LOCAL_SERVER_PORT: &str = "local_server_port";
pub(crate) const KEY_LOCAL_SERVER_HOST: &str = "local_server_host";

/// 内置服务器默认端口（里程碑 R4）：**刻意不等于** `DEFAULT_PORT`（42420）——传输层已经
/// 占了那个号的 TCP 与 UDP，内置服务器同样两个都要。
pub(crate) const DEFAULT_LOCAL_SERVER_PORT: u16 = 42421;

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
    plugin_ids: &[&str],
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
        server_url: get_json(store, KEY_SERVER_URL).await?,
        enable_remote: get_json(store, KEY_ENABLE_REMOTE).await?.unwrap_or(false),
        // 默认 true：见 `Settings::enable_port_mapping` 的文档——外层 `enable_remote`
        // 默认关闭已经保证了「不打开就不出网」，这一项是在那之内的取舍。
        enable_port_mapping: get_json(store, KEY_ENABLE_PORT_MAPPING)
            .await?
            .unwrap_or(true),
        // 默认关闭：内置服务器会对外开一个长期监听的端口，属于用户要主动选择的事。
        enable_local_server: get_json(store, KEY_ENABLE_LOCAL_SERVER)
            .await?
            .unwrap_or(false),
        local_server_port: get_json(store, KEY_LOCAL_SERVER_PORT)
            .await?
            .unwrap_or(DEFAULT_LOCAL_SERVER_PORT),
        local_server_host: get_json(store, KEY_LOCAL_SERVER_HOST).await?,
        plugins: load_plugin_settings(store, plugin_ids).await,
    })
}

/// 只要核心那几项，不读插件的。
///
/// 后台循环（端口映射、服务器注册）每轮都要看一眼 `enable_remote` 之类的开关，
/// 它们对插件设置没有任何兴趣；显式走这个入口比到处传一个空数组更说明意图。
pub(crate) async fn load_core(
    store: &Store,
    fallback_name: &str,
    fallback_save_dir: &str,
) -> Result<Settings> {
    load(store, fallback_name, fallback_save_dir, &[]).await
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
    set_json(store, KEY_ENABLE_LOCAL_SERVER, &s.enable_local_server).await?;
    set_json(store, KEY_LOCAL_SERVER_PORT, &s.local_server_port).await?;
    set_json(store, KEY_LOCAL_SERVER_HOST, &s.local_server_host).await?;
    // 插件的设置：一个插件一条记录，键名 `plugin.<id>`，内容是插件自己的 JSON。
    // **核心不解释里面有什么**——那正是 F1 划出插件边界要买的东西。
    for (id, value) in &s.plugins {
        set_json(store, &plugin_key(id), value).await?;
    }
    Ok(())
}

/// 插件设置在 `settings` 表里的键名。
fn plugin_key(id: &str) -> String {
    format!("plugin.{id}")
}

/// 读各插件自己的设置。
///
/// 只读**本次构建真的装了的**插件——`plugin_ids` 来自 `PluginRegistry`。没装的插件
/// 即使库里还留着它的旧设置也不读出来，免得前端画出一个点了没反应的表单。
async fn load_plugin_settings(
    store: &Store,
    plugin_ids: &[&str],
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for id in plugin_ids {
        // 读不出来（库出错 / 还没写过）就给个 `null`——插件侧一律
        // `serde_json::from_value(..).unwrap_or_default()`，缺字段回落默认值。
        let value = get_json::<serde_json::Value>(store, &plugin_key(id))
            .await
            .ok()
            .flatten()
            .unwrap_or(serde_json::Value::Null);
        map.insert((*id).to_string(), value);
    }
    map
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

/// 只读一个 `enable_remote`，不去 `load()` 整份设置。
///
/// 调用点是每 30 秒一轮、对每台设备都跑的可达性归类（`reach::classify`），
/// 那里只需要知道「本机出不出网」这一个事实；为它把 28 个字段全读一遍是浪费。
/// 读不到（库出错）时按 `false` 算——保守方向：宁可告诉用户「远程连接没开」
/// 让他去检查，也不要说「试过了连不上」把他引到错的地方排查。
pub(crate) async fn remote_enabled(store: &Store) -> bool {
    get_json::<bool>(store, KEY_ENABLE_REMOTE)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}
