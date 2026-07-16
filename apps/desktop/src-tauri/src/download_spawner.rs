//! `SidecarSpawner` 的 Tauri 实现：用 `tauri-plugin-shell` 的 sidecar 机制拉起
//! 打包的引擎二进制（DOWNLOAD_DESIGN.md §2/§3.1）。这是 `aa4c-download`
//! 依赖倒置边界在桌面壳层的具体实现——crate 本身不知道、也不需要知道背后是
//! Tauri（同 `RelayDialer`/`PunchDialer`/`ShareResolver` 一路的既有先例）。
//!
//! 一个实例绑定一个具体的 sidecar 名字（构造时指定，`"aria2c"` 或
//! `"transmission-daemon"`）——D1 上线时是硬编码 `"aria2c"`，D2 接
//! Transmission 时需要第二个不同名字的 sidecar，遂参数化。

use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

use aa4c_download::{EngineChild, KillFuture, SidecarSpawner, SpawnFuture};
use aa4c_types::Aa4cError;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

const STDIO_TAIL_LINES: usize = 20;

pub struct TauriSidecarSpawner {
    app: AppHandle,
    sidecar_name: String,
}

impl TauriSidecarSpawner {
    pub fn new(app: AppHandle, sidecar_name: impl Into<String>) -> Self {
        Self {
            app,
            sidecar_name: sidecar_name.into(),
        }
    }
}

impl SidecarSpawner for TauriSidecarSpawner {
    fn spawn(&self, args: &[String]) -> SpawnFuture {
        let app = self.app.clone();
        let sidecar_name = self.sidecar_name.clone();
        let args = args.to_vec();
        Box::pin(async move {
            let sidecar = app.shell().sidecar(&sidecar_name).map_err(|e| {
                Aa4cError::Unavailable(format!("{sidecar_name} sidecar not configured: {e}"))
            })?;
            let (mut rx, child) = sidecar.args(args).spawn().map_err(|e| {
                Aa4cError::Unavailable(format!("failed to spawn {sidecar_name} sidecar: {e}"))
            })?;
            let pid = child.pid();
            // 持续消费 stdout/stderr/terminate 事件——必须持续消费，否则内部有界
            // channel 满了会反过来阻塞子进程自己的输出写入。stdout+stderr 合并
            // 留最后几行（诊断用：健康检查反复失败但"进程看起来启动成功"时，
            // 唯一能解释原因的线索往往是引擎自己打印的日志——一次真实排查中
            // 发现 aria2 的 NOTICE/ERROR 日志实际上走 stdout 不是 stderr，只
            // 捕获 stderr 会看到一片空白，两路都要捕获）。
            let tail =
                std::sync::Arc::new(StdMutex::new(VecDeque::with_capacity(STDIO_TAIL_LINES)));
            let tail2 = tail.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let line = match event {
                        CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                            Some(String::from_utf8_lossy(&bytes).into_owned())
                        }
                        CommandEvent::Error(msg) => Some(msg),
                        _ => None,
                    };
                    if let Some(line) = line {
                        let mut guard = tail2.lock().unwrap_or_else(|e| e.into_inner());
                        if guard.len() >= STDIO_TAIL_LINES {
                            guard.pop_front();
                        }
                        guard.push_back(line);
                    }
                }
            });
            let handle: Box<dyn EngineChild> = Box::new(TauriEngineChild {
                child: Mutex::new(Some(child)),
                tail,
                pid,
            });
            Ok(handle)
        })
    }
}

struct TauriEngineChild {
    child: Mutex<Option<CommandChild>>,
    tail: std::sync::Arc<StdMutex<VecDeque<String>>>,
    pid: u32,
}

impl EngineChild for TauriEngineChild {
    fn kill(&self) -> KillFuture<'_> {
        Box::pin(async move {
            if let Some(child) = self.child.lock().await.take() {
                let _ = child.kill();
            }
        })
    }

    fn recent_stdio(&self) -> Vec<String> {
        self.tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}
