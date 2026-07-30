//! 子进程生死管理（DOWNLOAD_DESIGN.md §2）。`SidecarSpawner` 只管拉起打包的
//! 引擎二进制、拿到一个能终止它的句柄——**不管通信**，数据面是回环 JSON-RPC
//! （见 `rpc.rs`），这是与其他依赖倒置 trait（`RelayDialer`/`PunchDialer`/
//! `ShareResolver`）的一个关键区别：那几个的 trait 方法本身就是数据面。
//!
//! `ProcessSpawner` 是 Docker/无 GUI headless 场景要用的真实实现（不是测试桩），
//! 桌面 Tauri 壳层的 sidecar 适配实现是另一个 `SidecarSpawner`（见 apps/desktop）。
//!
//! `spawn` 的参数是通用 `args: &[String]`（D1 上线时是"只传 `--conf-path=X`
//! 一个参数"的窄签名，D2 接 Transmission 时发现它的命令行形状是
//! `-f --config-dir=X` 两个参数，不是"单参数替换"能表达的，遂泛化）——具体
//! 参数数组由各自引擎的 spawn 封装层（`conf.rs` 之于 aria2、
//! `transmission_conf.rs` 之于 Transmission）决定，这个 trait 本身只管
//! "拉起可执行文件、透传参数"。一个 `SidecarSpawner` 实例绑定一个具体的
//! 引擎二进制（构造时指定名字/路径），不做"按名字多路复用"。

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
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
    /// 拉起引擎，`args` 是完整命令行参数数组（不含 argv[0]）。密钥等敏感选项
    /// 一律不通过 `args` 传（DOWNLOAD_DESIGN.md §3.1：命令行参数对本机任意
    /// 用户的进程经 `ps`/WMI 可见）——`args` 只应包含配置文件路径/公开标志位
    /// 这类不敏感的启动参数。
    ///
    /// `envs` 是额外注入的环境变量（AI2.2 新增，llama-server 需要）——与
    /// `args` 的取舍互补：aria2/Transmission 靠配置文件传参，唯一需要动态
    /// 环境变量的是 Transmission 的动态库搜索路径，且那是"壳层固定知道怎么
    /// 算"的静态信息，`TauriSidecarSpawner` 内部直接处理，不需要经这个参数；
    /// llama-server 反过来**全部配置都走环境变量**（含随机端口、随机
    /// `LLAMA_API_KEY`、模型路径——这些是调用方（`aa4c-ai`）每次 spawn 时才
    /// 知道的动态值，不可能让壳层内部猜到），所以这个参数是必需的，不是
    /// 对称性设计——两个引擎各自只用其中一条注入路径。
    fn spawn(&self, args: &[String], envs: &[(String, String)]) -> SpawnFuture;
}

/// [`EngineChild::kill`] 的返回：借用 `&self`（同一句柄可被多次 `kill` 幂等调用），
/// 生命周期绑定在借用上，调用方立即 `.await` 即可，不存在跨借用悬垂的风险。
pub type KillFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// 一个正在运行的引擎子进程句柄：只暴露"终止"、"最近的 stdio 输出"、"PID"，
/// 不暴露"如何与它通信"（数据面走回环 RPC，见 rpc.rs）。
pub trait EngineChild: Send + Sync {
    /// 强制终止。幂等、尽力而为——进程已退出时静默忽略，绝不 panic。
    fn kill(&self) -> KillFuture<'_>;
    /// 最近若干行 stdout+stderr（诊断用，合并成一份按行 tail）：健康检查/连接
    /// 反复失败但进程"看起来启动成功"时，唯一能解释原因的线索往往是引擎自己
    /// 打印的日志——之前用 `Stdio::null()` 全部丢弃，一次真实的 Windows CI 失败
    /// 排查中先只捕获了 stderr，结果是空的：aria2 的 NOTICE/ERROR 日志实际上走
    /// **stdout**（同 `aria2c --conf-path=...` 直接跑起来能看到的输出一致），
    /// 两路都要捕获才不会又漏掉真正有信息量的那一路。
    fn recent_stdio(&self) -> Vec<String>;
    /// 子进程 PID——孤儿进程防护用（`orphan_guard`）：Windows 的 Job Object
    /// 归属、macOS/Tauri-Linux 的 PID 文件方案都需要事后按 PID 操作，
    /// 不依赖 spawn 时能注入钩子（`tauri_plugin_shell::process::CommandChild`
    /// 恰好就是这种"只给 PID，不给别的"的句柄，见 apps/desktop 的实现）。
    fn pid(&self) -> u32;
}

const STDIO_TAIL_LINES: usize = 20;

/// 起一个后台任务把 `reader`（子进程的 stdout 或 stderr）按行读进共享 tail
/// 缓冲区（两路共用同一个缓冲区、按到达顺序合并，诊断时不需要分别看两份）。
fn spawn_tail_reader(
    reader: impl tokio::io::AsyncRead + Send + Unpin + 'static,
    tail: std::sync::Arc<StdMutex<VecDeque<String>>>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
            if guard.len() >= STDIO_TAIL_LINES {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    });
}

/// `tokio::process` 实现：显式二进制路径或 PATH 裸命令名（如 `"aria2c"`）。
/// D1 的开发/测试/CI 与将来 Docker/headless 场景共用这一个实现——见
/// V0.4_IMPLEMENTATION_PLAN.md D1 的排序说明（引擎二进制打包放在代码之后）。
///
/// Linux 上无条件给每个子进程装 `PR_SET_PDEATHSIG`（`orphan_guard::linux`）：
/// 对 aria2 是免费的额外保险（它自己已经有 `stop-with-process`），对
/// Transmission（没有等价内建选项，DOWNLOAD_DESIGN.md §3.6.2）是必需的。
/// 只有这条路径能用——`pre_exec` 必须在 spawn 时注入，Tauri sidecar 机制
/// 事后拿不到这个钩子，见 `orphan_guard` 模块文档。
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
    fn spawn(&self, args: &[String], envs: &[(String, String)]) -> SpawnFuture {
        let binary = self.binary.clone();
        let args = args.to_vec();
        let envs = envs.to_vec();
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&binary);
            cmd.args(&args)
                .envs(envs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            crate::orphan_guard::arm_pdeathsig(&mut cmd);
            let mut child = cmd.spawn().map_err(|e| {
                Aa4cError::Unavailable(format!("failed to spawn {}: {e}", binary.display()))
            })?;

            let pid = child.id().ok_or_else(|| {
                Aa4cError::Unavailable(format!(
                    "{} exited immediately after spawn (no pid)",
                    binary.display()
                ))
            })?;

            let tail =
                std::sync::Arc::new(StdMutex::new(VecDeque::with_capacity(STDIO_TAIL_LINES)));
            if let Some(stdout) = child.stdout.take() {
                spawn_tail_reader(stdout, tail.clone());
            }
            if let Some(stderr) = child.stderr.take() {
                spawn_tail_reader(stderr, tail.clone());
            }

            let handle: Box<dyn EngineChild> = Box::new(ProcessChild {
                child: Mutex::new(Some(child)),
                tail,
                pid,
            });
            Ok(handle)
        })
    }
}

struct ProcessChild {
    child: Mutex<Option<tokio::process::Child>>,
    tail: std::sync::Arc<StdMutex<VecDeque<String>>>,
    pid: u32,
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
