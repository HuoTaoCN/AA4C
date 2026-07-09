//! `SidecarSpawner` 的 Tauri 实现：用 `tauri-plugin-shell` 的 sidecar 机制拉起
//! 打包的 aria2c 二进制（DOWNLOAD_DESIGN.md §2/§3.1）。这是 `aa4c-download`
//! 依赖倒置边界在桌面壳层的具体实现——crate 本身不知道、也不需要知道背后是
//! Tauri（同 `RelayDialer`/`PunchDialer`/`ShareResolver` 一路的既有先例）。

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex as StdMutex;

use aa4c_download::{EngineChild, KillFuture, SidecarSpawner, SpawnFuture};
use aa4c_types::Aa4cError;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

const STDERR_TAIL_LINES: usize = 20;

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
            // 持续消费 stdout/stderr/terminate 事件——必须持续消费，否则内部有界
            // channel 满了会反过来阻塞子进程自己的输出写入。stderr 留最后几行
            // （诊断用：健康检查反复失败但"进程看起来启动成功"时，唯一能解释
            // 原因的线索往往是引擎自己打印的错误，之前全部丢弃过一次真实的
            // Windows 端 aria2 conf 解析失败，只剩一个无信息量的连接被拒错误）。
            let stderr_tail =
                std::sync::Arc::new(StdMutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    if let CommandEvent::Stderr(bytes) = event {
                        let line = String::from_utf8_lossy(&bytes).into_owned();
                        let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
                        if guard.len() >= STDERR_TAIL_LINES {
                            guard.pop_front();
                        }
                        guard.push_back(line);
                    }
                }
            });
            let handle: Box<dyn EngineChild> = Box::new(TauriEngineChild {
                child: Mutex::new(Some(child)),
                stderr_tail,
            });
            Ok(handle)
        })
    }
}

struct TauriEngineChild {
    child: Mutex<Option<CommandChild>>,
    stderr_tail: std::sync::Arc<StdMutex<VecDeque<String>>>,
}

impl EngineChild for TauriEngineChild {
    fn kill(&self) -> KillFuture<'_> {
        Box::pin(async move {
            if let Some(child) = self.child.lock().await.take() {
                let _ = child.kill();
            }
        })
    }

    fn recent_stderr(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}
