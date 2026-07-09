//! `SidecarSpawner` 的 Tauri 实现：用 `tauri-plugin-shell` 的 sidecar 机制拉起
//! 打包的 aria2c 二进制（DOWNLOAD_DESIGN.md §2/§3.1）。这是 `aa4c-download`
//! 依赖倒置边界在桌面壳层的具体实现——crate 本身不知道、也不需要知道背后是
//! Tauri（同 `RelayDialer`/`PunchDialer`/`ShareResolver` 一路的既有先例）。

use std::path::Path;

use aa4c_download::{EngineChild, KillFuture, SidecarSpawner, SpawnFuture};
use aa4c_types::Aa4cError;
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

pub struct TauriSidecarSpawner {
    app: AppHandle,
}

impl TauriSidecarSpawner {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl SidecarSpawner for TauriSidecarSpawner {
    fn spawn(&self, conf_path: &Path) -> SpawnFuture {
        let app = self.app.clone();
        let arg = format!("--conf-path={}", conf_path.display());
        Box::pin(async move {
            let sidecar = app.shell().sidecar("aria2c").map_err(|e| {
                Aa4cError::Unavailable(format!("aria2c sidecar not configured: {e}"))
            })?;
            let (mut rx, child) = sidecar.args([arg]).spawn().map_err(|e| {
                Aa4cError::Unavailable(format!("failed to spawn aria2c sidecar: {e}"))
            })?;
            // 持续消费 stdout/stderr/terminate 事件并丢弃——同 `ProcessSpawner` 的
            // `Stdio::null()` 效果一致；必须持续消费，否则内部有界 channel 满了
            // 会反过来阻塞子进程自己的输出写入。
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            let handle: Box<dyn EngineChild> = Box::new(TauriEngineChild {
                child: Mutex::new(Some(child)),
            });
            Ok(handle)
        })
    }
}

struct TauriEngineChild {
    child: Mutex<Option<CommandChild>>,
}

impl EngineChild for TauriEngineChild {
    fn kill(&self) -> KillFuture<'_> {
        Box::pin(async move {
            if let Some(child) = self.child.lock().await.take() {
                let _ = child.kill();
            }
        })
    }
}
