//! AA4C 桌面端 Tauri 后端：API_DESIGN.md §9 的 11 个 Command 与事件转发。
//!
//! 本层只做参数搬运、错误映射（`{ code, message }`）与事件桥接；
//! 业务编排全部在 `aa4c_core::Core`（同时服务桌面端与 Android）。

mod commands;
mod download_spawner;

use std::sync::Arc;

use aa4c_core::{Core, CoreConfig};
use aa4c_types::CoreEvent;
use tauri::{Emitter, Manager};
use tokio::sync::broadcast::error::RecvError;
use tracing_subscriber::EnvFilter;

/// 把一个 `CoreEvent` 转发为 Tauri 事件（`aa4c://` + 蛇形名，payload 见 §9.2）。
fn forward_event(app: &tauri::AppHandle, event: &CoreEvent) {
    let name = format!("aa4c://{}", event.event_name());
    if let Err(e) = app.emit(&name, commands::event_payload(event)) {
        tracing::warn!(event = name, error = %e, "failed to emit event to webview");
    }
}

/// 订阅事件总线并持续转发到前端，直至 Core 关闭。
fn spawn_event_forwarder(app: tauri::AppHandle, core: Arc<Core>) {
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => forward_event(&app, &event),
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event forwarder lagged behind");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// 桌面三平台注入基于 `tauri-plugin-shell` 的下载引擎子进程拉起器；Android 等
/// 移动构建返回 `None`——下载能力整体不存在，`aa4c_core::orchestrate` 侧的
/// 下载相关 Command 会统一报 `Unavailable`（V0.4 范围决定，见 DOWNLOAD_DESIGN.md §1.1）。
#[cfg(desktop)]
fn desktop_download_spawner(
    app: &tauri::AppHandle,
) -> Option<Arc<dyn aa4c_download::SidecarSpawner>> {
    Some(Arc::new(download_spawner::TauriSidecarSpawner::new(
        app.clone(),
        "aria2c",
    )))
}

#[cfg(not(desktop))]
fn desktop_download_spawner(
    _app: &tauri::AppHandle,
) -> Option<Arc<dyn aa4c_download::SidecarSpawner>> {
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(version = aa4c_core::VERSION, "AA4C desktop starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // 数据目录由 Tauri 注入（桌面为应用数据目录，Android 为应用私有目录）。
            // 联调钩子：AA4C_DATA_DIR 覆盖数据目录（含接收目录），使同一台机器能跑
            // 多个隔离实例做双机联调；AA4C_DEVICE_NAME 指定首启设备名便于区分窗口。
            let override_dir = std::env::var("AA4C_DATA_DIR")
                .ok()
                .map(std::path::PathBuf::from);
            let data_dir = match &override_dir {
                Some(dir) => dir.clone(),
                None => app.path().app_data_dir()?,
            };
            // 接收目录：优先系统下载目录，取不到（如 Android）回落到应用数据目录；
            // 数据目录被覆盖时接收目录跟着进去，保证实例之间 Inbox 也互相隔离
            let save_dir = match &override_dir {
                Some(dir) => dir.join("Inbox"),
                None => app
                    .path()
                    .download_dir()
                    .unwrap_or_else(|_| data_dir.clone())
                    .join("AA4C"),
            };
            let mut config = CoreConfig::new(data_dir);
            config.transfer.default_save_dir = save_dir;
            if let Ok(name) = std::env::var("AA4C_DEVICE_NAME") {
                config.device_name = Some(name);
            }
            // 下载中心（DOWNLOAD_DESIGN.md，里程碑 D1）：只在桌面三平台注入 sidecar
            // 拉起器——V0.4 明确不含 Android，`cfg(desktop)` 由 Tauri 自动区分。
            config.download_spawner = desktop_download_spawner(app.handle());

            // 启动序列是异步的；setup 在事件循环前运行，可阻塞等待
            let core = tauri::async_runtime::block_on(Core::start(config))?;

            spawn_event_forwarder(app.handle().clone(), core.clone());
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_self_device,
            commands::list_devices,
            commands::start_pairing,
            commands::confirm_pairing,
            commands::unpair_device,
            commands::set_trust_level,
            commands::send_files,
            commands::accept_transfer,
            commands::cancel_transfer,
            commands::list_transfers,
            commands::get_settings,
            commands::update_settings,
            commands::list_sync_scopes,
            commands::add_sync_scope,
            commands::remove_sync_scope,
            commands::list_sync_files,
            commands::rescan_sync,
            commands::list_unified_files,
            commands::refresh_remote_index,
            commands::fetch_file,
            commands::list_conflicts,
            commands::create_share,
            commands::list_shares,
            commands::revoke_share,
            commands::list_share_access,
            commands::open_share,
            commands::add_download,
            commands::pause_download,
            commands::resume_download,
            commands::cancel_download,
            commands::list_downloads,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
