//! AA4C 桌面端 Tauri 后端：API_DESIGN.md §9 的 11 个 Command 与事件转发。
//!
//! 本层只做参数搬运、错误映射（`{ code, message }`）与事件桥接；
//! 业务编排全部在 `aa4c_core::Core`（同时服务桌面端与 Android）。

mod commands;

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
        .setup(|app| {
            // 数据目录由 Tauri 注入（桌面为应用数据目录，Android 为应用私有目录）
            let data_dir = app.path().app_data_dir()?;
            // 接收目录：优先系统下载目录，取不到（如 Android）回落到应用数据目录
            let save_dir = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| data_dir.clone())
                .join("AA4C");
            let mut config = CoreConfig::new(data_dir);
            config.transfer.default_save_dir = save_dir;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
