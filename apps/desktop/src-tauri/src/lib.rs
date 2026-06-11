//! AA4C 桌面端 Tauri 后端。
//!
//! M6 里程碑将在此实现 API_DESIGN.md §9 的全部 Command 与事件转发；
//! 当前仅提供占位 Command 用于验证前后端通路。

use tracing_subscriber::EnvFilter;

/// 占位 Command：验证前后端 IPC 通路（M6 时替换为正式 Command 集）。
#[tauri::command]
fn aa4c_version() -> String {
    aa4c_core::VERSION.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(version = aa4c_core::VERSION, "AA4C desktop starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![aa4c_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
