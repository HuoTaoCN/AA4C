//! 子进程生死管理（DOWNLOAD_DESIGN.md §2）。`SidecarSpawner` 只管拉起打包的
//! 引擎二进制、拿到一个能终止它的句柄——**不管通信**，数据面是回环 JSON-RPC
//! （见 `rpc.rs`），这是与其他依赖倒置 trait（`RelayDialer`/`PunchDialer`/
//! `ShareResolver`）的一个关键区别：那几个的 trait 方法本身就是数据面。
//!
//! `ProcessSpawner` 是 Docker/无 GUI headless 场景要用的真实实现（不是测试桩），
//! 桌面 Tauri 壳层的 sidecar 适配实现是另一个 `SidecarSpawner`（见 apps/desktop）。

use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Mutex as StdMutex;

use aa4c_types::{Aa4cError, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

/// [`SidecarSpawner::spawn`] 的返回（避免引入 async-trait 依赖，同 aa4c-transfer
/// 的 `ResolveFuture`/`RelayDialFuture` 先例）。
pub type SpawnFuture = Pin<Box<dyn Future<Output = Result<Box<dyn EngineChild>>> + Send>>;

/// 拉起打包的引擎二进制，具体实现由壳层注入（Tauri sidecar / `ProcessSpawner`）。
pub trait SidecarSpawner: Send + Sync + 'static {
    /// 拉起引擎，命令行**只**传 `--conf-path=<conf_path>`（DOWNLOAD_DESIGN.md §3.1：
    /// 密钥等全部选项都在 conf 文件里，收敛命令行形状是刻意的）。
    fn spawn(&self, conf_path: &Path) -> SpawnFuture;
}

/// [`EngineChild::kill`] 的返回：借用 `&self`（同一句柄可被多次 `kill` 幂等调用），
/// 生命周期绑定在借用上，调用方立即 `.await` 即可，不存在跨借用悬垂的风险。
pub type KillFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// 一个正在运行的引擎子进程句柄：只暴露"终止"和"最近的 stderr 输出"，不暴露
/// "如何与它通信"（数据面走回环 RPC，见 rpc.rs）。
pub trait EngineChild: Send + Sync {
    /// 强制终止。幂等、尽力而为——进程已退出时静默忽略，绝不 panic。
    fn kill(&self) -> KillFuture<'_>;
    /// 最近若干行 stderr（诊断用）：健康检查/连接反复失败但进程"看起来启动
    /// 成功"时，唯一能解释原因的线索往往是引擎自己打印的错误——之前只
    /// `Stdio::null()` 全部丢弃，Windows CI 上一次真实的 aria2 conf 解析失败
    /// 就是这样被吞掉、只剩一个无信息量的"connection refused"。
    fn recent_stderr(&self) -> Vec<String>;
}

const STDERR_TAIL_LINES: usize = 20;

/// `tokio::process` 实现：显式二进制路径或 PATH 裸命令名（如 `"aria2c"`）。
/// D1 的开发/测试/CI 与将来 Docker/headless 场景共用这一个实现——见
/// V0.4_IMPLEMENTATION_PLAN.md D1 的排序说明（引擎二进制打包放在代码之后）。
pub struct ProcessSpawner {
    binary: PathBuf,
}

impl ProcessSpawner {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

impl SidecarSpawner for ProcessSpawner {
    fn spawn(&self, conf_path: &Path) -> SpawnFuture {
        let binary = self.binary.clone();
        let arg = format!("--conf-path={}", conf_path.display());
        Box::pin(async move {
            let mut child = tokio::process::Command::new(&binary)
                .arg(arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| {
                    Aa4cError::Unavailable(format!("failed to spawn {}: {e}", binary.display()))
                })?;

            let stderr_tail =
                std::sync::Arc::new(StdMutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
            if let Some(stderr) = child.stderr.take() {
                let tail = stderr_tail.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
                        if guard.len() >= STDERR_TAIL_LINES {
                            guard.pop_front();
                        }
                        guard.push_back(line);
                    }
                });
            }

            let handle: Box<dyn EngineChild> = Box::new(ProcessChild {
                child: Mutex::new(Some(child)),
                stderr_tail,
            });
            Ok(handle)
        })
    }
}

struct ProcessChild {
    child: Mutex<Option<tokio::process::Child>>,
    stderr_tail: std::sync::Arc<StdMutex<VecDeque<String>>>,
}

impl EngineChild for ProcessChild {
    fn kill(&self) -> KillFuture<'_> {
        Box::pin(async move {
            let mut guard = self.child.lock().await;
            if let Some(child) = guard.as_mut() {
                let _ = child.kill().await;
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
