//! 下载中心服务（V0.4 里程碑 D1/D2，DOWNLOAD_DESIGN.md）。
//!
//! 独立于设备身份/配对——下载没有"对端设备"概念。对上层（`aa4c-core`）只暴露
//! 统一的任务模型（`add`/`pause`/`resume`/`cancel`/`list`），引擎细节（aria2 子
//! 进程 + JSON-RPC，D1；transmission-daemon 子进程 + HTTP RPC，D2）全部封装在
//! 这个 crate 内部——调用方不需要知道一条 `magnet:` 链接背后连的是哪个进程。
//!
//! 内部是**两个独立的单线程 actor**（aria2 一个、Transmission 一个），各自
//! 独占持有自己的"当前连接"状态（子进程句柄 + RPC 客户端），公开方法按 URL
//! scheme（`add`）或任务 id 形状（`pause`/`resume`/`cancel`，见
//! `is_bt_task_id`）分流到对应 actor 的 channel——两个引擎的生命周期
//! 互不影响：aria2 不可用不妨碍 BT 下载正常工作，反之亦然。“服务当前不可用”
//! 对每个引擎各自有一个天然、单一的判定点（对应 channel 发送失败 = 那个
//! actor 已退出 = 那个引擎不可用）。

// D2 接孤儿进程防护要用到的 unsafe FFI（Win32 Job Object / Linux `prctl`）已经
// 随 AI2.1 平移进 `aa4c-engine`——这个 crate 自身不再有 unsafe 代码，恢复到
// D1 上线时的 `forbid`。
#![forbid(unsafe_code)]

mod conf;
mod rpc;
mod transmission_conf;
mod transmission_process;
mod transmission_rpc;
mod util;

pub use aa4c_engine::{EngineChild, KillFuture, ProcessSpawner, SidecarSpawner, SpawnFuture};
pub use rpc::{Aria2Client, Aria2Notification};
pub use transmission_process::TransmissionProcess;
pub use transmission_rpc::TransmissionClient;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aa4c_store::Store;
use aa4c_types::{Aa4cError, CoreEvent, DownloadKind, DownloadStatus, Result, TaskId};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

/// 端口被抢/健康检查失败时的整体重试次数（每次换一个新端口）。
const SPAWN_ATTEMPTS: u32 = 3;
/// 单次尝试内，等待 aria2c 就绪（`getVersion` 探测）的重试次数与递增退避。
const HEALTH_CHECK_ATTEMPTS: u32 = 8;
const HEALTH_CHECK_BASE_DELAY: Duration = Duration::from_millis(200);
/// WS 连接掉线后的重连尝试次数（同一进程，同一端口/密钥——不重启子进程）。
const RECONNECT_ATTEMPTS: u32 = 5;
/// 事件通知/进度轮询/DB 节流写入统一走的一个节拍——同时充当"进行中任务的进度
/// 广播频率"与"漏事件兜底对账频率"（DOWNLOAD_DESIGN.md §3.2/§4 的两处"数秒级"
/// 由这一个定时器统一实现，不另起第二套节流簿记）。
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);
/// 优雅关闭：`aria2.shutdown` 之后留给进程自行退出的宽限期，超时强杀。
/// aria2 的 `shutdown`（区别于 `forceShutdown`）本身会等**约 3 秒**让活跃下载
/// 收尾再真正退出（实测确认：日志打「3 second(s) has passed. Stopping
/// application.」）——这个宽限期必须明显长于那 3 秒，否则我们自己的强杀会抢在
/// aria2 完成 session 落盘之前发生，直接违背"先礼后兵"想要的效果。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// Transmission 的 RPC 是纯请求-响应（HTTP，每次调用开关一条连接），没有 aria2
/// WS 连接那种"断开"信号可以监听——`torrent-get` 轮询本身就是唯一的存活信号。
/// 连续这么多次轮询失败（daemon 进程可能已经崩溃）才判定 BT 能力在本次会话
/// 剩余时间内不可用，避免偶发的单次网络抖动就整个宣布降级。
const BT_FAILURE_THRESHOLD: u32 = 5;

type EventSender = broadcast::Sender<CoreEvent>;

enum Cmd {
    Add {
        url: String,
        reply: oneshot::Sender<Result<TaskId>>,
    },
    Pause {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Resume {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Cancel {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    /// 批量操作"清除已完成记录"（D3）专用——只让引擎忘掉这个已完成任务（不删
    /// 本地文件），**不碰 DB**：调用方（`clear_completed`）要先确认这个调用成功
    /// 了才会删对应的 DB 行，跟 `Cancel` 那种"不关心 RPC 成不成功，DB 是唯一
    /// 事实来源"的取舍刻意不同——原因见 `DownloadService::clear_completed` 文档。
    ForgetCompleted {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// BT（Transmission）actor 的命令——形状同 `Cmd`，`Add` 换成接收 magnet URI。
enum BtCmd {
    Add {
        magnet: String,
        reply: oneshot::Sender<Result<TaskId>>,
    },
    Pause {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Resume {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Cancel {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    /// 同 `Cmd::ForgetCompleted`：只让引擎忘掉这个任务，不碰 DB。
    ForgetCompleted {
        id: TaskId,
        reply: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// aria2 GID 固定 16 位十六进制、BT infohash（SHA-1）固定 40 位十六进制——
/// 两种引擎原生 id 的固定长度不同是协议本身决定的，不是巧合，用长度判断一个
/// 任务 id 该路由去哪个 actor 比额外查一次数据库拿 `kind` 更省一次往返
/// （`pause`/`resume`/`cancel` 都要用，热路径）。
fn is_bt_task_id(id: &str) -> bool {
    id.len() == 40
}

/// 下载中心服务句柄——统一封装 aria2（D1，HTTP/HTTPS/FTP）与 Transmission
/// （D2，BT/Magnet）两个独立 actor，调用方不需要关心一个任务 id 具体是哪个
/// 引擎在管。任一 `cmd_tx` 为 `None`（对应引擎启动失败）或后台 actor 已退出
/// （BT 侧轮询连续失败耗尽/已 shutdown）时，路由到那个引擎的操作性方法统一
/// 返回 `Aa4cError::Unavailable`——两个引擎的可用性完全独立。
pub struct DownloadService {
    store: Store,
    cmd_tx: Option<mpsc::UnboundedSender<Cmd>>,
    bt_cmd_tx: Option<mpsc::UnboundedSender<BtCmd>>,
}

/// 限速/并发/BT 分享率/BT 空闲做种超时（D3，DOWNLOAD_DESIGN.md §9）——`None`
/// 表示不设限，用引擎自己的默认行为。打包成一个结构体而不是四个独立 `Option`
/// 参数，避免 `start()` 调用点因为参数顺序相似而传错位置。
#[derive(Debug, Clone, Default)]
pub struct DownloadLimits {
    pub speed_limit_kbps: Option<u32>,
    pub concurrency: Option<u32>,
    pub bt_ratio_limit: Option<f64>,
    pub bt_idle_seeding_limit_minutes: Option<u32>,
}

impl DownloadService {
    /// 启动：并行拉起 aria2c 与 transmission-daemon（`bt_spawner` 为 `None`
    /// 时 BT 能力整体不存在，同 `spawner` 为 `None` 时 HTTP 能力不存在——两者
    /// 互不阻塞，一个引擎起不来不影响另一个）。**启动失败不返回 `Err`**——
    /// 下载能力整体降级不可用，但不阻塞调用方（同 QUIC 端点等既有可选能力的
    /// 一贯降级设计，DOWNLOAD_DESIGN.md §3.1/§3.6）。
    pub async fn start(
        spawner: Arc<dyn SidecarSpawner>,
        bt_spawner: Option<Arc<dyn SidecarSpawner>>,
        store: Store,
        events: EventSender,
        data_dir: PathBuf,
        download_dir: PathBuf,
        limits: DownloadLimits,
    ) -> Arc<Self> {
        let cmd_tx = match spawn_and_connect_with_retries(
            spawner.as_ref(),
            &data_dir,
            &download_dir,
            limits.speed_limit_kbps,
            limits.concurrency,
        )
        .await
        {
            Ok(connected) => {
                reconcile(&store, &events, &connected.client).await;
                tracing::info!(port = connected.port, "download engine (aria2c) connected");
                let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
                tokio::spawn(actor_loop(store.clone(), events.clone(), connected, cmd_rx));
                Some(cmd_tx)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "aria2 unavailable at startup; HTTP/FTP download capability disabled for this session"
                );
                None
            }
        };

        let bt_cmd_tx = match bt_spawner {
            None => None,
            Some(bt_spawner) => {
                match spawn_and_connect_bt_with_retries(
                    bt_spawner.as_ref(),
                    &data_dir,
                    &download_dir,
                    limits.speed_limit_kbps,
                    limits.concurrency,
                    limits.bt_ratio_limit,
                    limits.bt_idle_seeding_limit_minutes,
                )
                .await
                {
                    Ok(connected) => {
                        let _ = bt_reconcile(&store, &events, &connected.client).await;
                        tracing::info!(
                            port = connected.port,
                            "download engine (transmission-daemon) connected"
                        );
                        let (bt_cmd_tx, bt_cmd_rx) = mpsc::unbounded_channel();
                        tokio::spawn(bt_actor_loop(store.clone(), events, connected, bt_cmd_rx));
                        Some(bt_cmd_tx)
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "transmission-daemon unavailable at startup; BT download capability disabled for this session"
                        );
                        None
                    }
                }
            }
        };

        Arc::new(Self {
            store,
            cmd_tx,
            bt_cmd_tx,
        })
    }

    fn unavailable() -> Aa4cError {
        Aa4cError::Unavailable("download engine not available".into())
    }

    /// 新建一条下载任务：`magnet:` 链接路由给 Transmission（D2），其余（HTTP/
    /// HTTPS/FTP 直链，D1）路由给 aria2——调用方不需要自己判断该走哪个引擎。
    pub async fn add(&self, url: String) -> Result<TaskId> {
        if url.starts_with("magnet:") {
            let tx = self.bt_cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
            let (reply, rx) = oneshot::channel();
            tx.send(BtCmd::Add { magnet: url, reply })
                .map_err(|_| Self::unavailable())?;
            return rx.await.map_err(|_| Self::unavailable())?;
        }
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Add { url, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn pause(&self, id: TaskId) -> Result<()> {
        if is_bt_task_id(&id) {
            let tx = self.bt_cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
            let (reply, rx) = oneshot::channel();
            tx.send(BtCmd::Pause { id, reply })
                .map_err(|_| Self::unavailable())?;
            return rx.await.map_err(|_| Self::unavailable())?;
        }
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Pause { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn resume(&self, id: TaskId) -> Result<()> {
        if is_bt_task_id(&id) {
            let tx = self.bt_cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
            let (reply, rx) = oneshot::channel();
            tx.send(BtCmd::Resume { id, reply })
                .map_err(|_| Self::unavailable())?;
            return rx.await.map_err(|_| Self::unavailable())?;
        }
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Resume { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn cancel(&self, id: TaskId) -> Result<()> {
        if is_bt_task_id(&id) {
            let tx = self.bt_cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
            let (reply, rx) = oneshot::channel();
            tx.send(BtCmd::Cancel { id, reply })
                .map_err(|_| Self::unavailable())?;
            return rx.await.map_err(|_| Self::unavailable())?;
        }
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Cancel { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    /// 按创建时间倒序列出全部下载任务（D1+D2 同一张表，同一个列表——D3「统一
    /// 任务中心」的目标从数据模型上一开始就成立）。数据库读取，不经过 actor
    /// （列表展示不需要强一致于"引擎当前是否在线"，引擎不可用时仍应能看到
    /// 历史记录）。
    pub async fn list(&self) -> Result<Vec<aa4c_types::DownloadTask>> {
        self.store.list_downloads().await
    }

    /// 批量操作（D3，DOWNLOAD_DESIGN.md §6/§9）：全部暂停/全部继续/清除已完成
    /// 记录。三个方法都是"尽力而为"——单个任务失败只跳过它，不中断整体、不让
    /// 一个任务的问题拖累其余任务；返回值是"实际成功的数量"，不是"尝试的数量"。
    /// 直接复用已有的单任务 `pause`/`resume`（本身就按 id 长度分流到对应引擎），
    /// 不需要新的 actor 命令。
    pub async fn pause_all(&self) -> usize {
        let tasks = self.store.list_downloads().await.unwrap_or_default();
        let mut ok = 0;
        for task in tasks {
            if matches!(
                task.status,
                DownloadStatus::Active | DownloadStatus::Waiting
            ) && self.pause(task.id).await.is_ok()
            {
                ok += 1;
            }
        }
        ok
    }

    pub async fn resume_all(&self) -> usize {
        let tasks = self.store.list_downloads().await.unwrap_or_default();
        let mut ok = 0;
        for task in tasks {
            if task.status == DownloadStatus::Paused && self.resume(task.id).await.is_ok() {
                ok += 1;
            }
        }
        ok
    }

    /// 清除已完成记录——**不是**简单删 DB 行。BT 任务完成后仍在做种（设计
    /// 故意如此，DOWNLOAD_DESIGN.md §3.6.4），只删记录不通知引擎的话，下一次
    /// `bt_reconcile()` 轮询会把"引擎有、表里没有"的它当孤儿记录补插回去，
    /// 清除操作形同虚设。所以要先让引擎"忘记"每个任务（`Cmd`/`BtCmd` 的
    /// `ForgetCompleted`——不删本地文件，只是让引擎不再持有/汇报这个任务），
    /// 成功的才删对应的 DB 行；单个任务引擎调用失败就跳过它，不中断整体。
    pub async fn clear_completed(&self) -> Result<usize> {
        let tasks = self.store.list_downloads().await?;
        let mut cleared_ids = Vec::new();
        for task in tasks {
            if task.status != DownloadStatus::Complete {
                continue;
            }
            let forgotten = if is_bt_task_id(&task.id) {
                match &self.bt_cmd_tx {
                    Some(tx) => {
                        let (reply, rx) = oneshot::channel();
                        tx.send(BtCmd::ForgetCompleted {
                            id: task.id.clone(),
                            reply,
                        })
                        .is_ok()
                            && matches!(rx.await, Ok(Ok(())))
                    }
                    None => false,
                }
            } else {
                match &self.cmd_tx {
                    Some(tx) => {
                        let (reply, rx) = oneshot::channel();
                        tx.send(Cmd::ForgetCompleted {
                            id: task.id.clone(),
                            reply,
                        })
                        .is_ok()
                            && matches!(rx.await, Ok(Ok(())))
                    }
                    None => false,
                }
            };
            if forgotten {
                cleared_ids.push(task.id);
            }
        }
        self.store.delete_completed_downloads(&cleared_ids).await?;
        Ok(cleared_ids.len())
    }

    /// 优雅关闭两个引擎（互不影响，一个失败不阻塞另一个）：
    /// aria2 `aria2.shutdown`（触发一次 session 保存）→ 宽限期 → 强杀；
    /// Transmission `session-close` → 宽限期 → 强杀。引擎本就不可用时是 no-op。
    pub async fn shutdown(&self) {
        if let Some(tx) = &self.cmd_tx {
            let (reply, rx) = oneshot::channel();
            if tx.send(Cmd::Shutdown { reply }).is_ok() {
                let _ = rx.await;
            }
        }
        if let Some(tx) = &self.bt_cmd_tx {
            let (reply, rx) = oneshot::channel();
            if tx.send(BtCmd::Shutdown { reply }).is_ok() {
                let _ = rx.await;
            }
        }
    }
}

struct Connected {
    child: Box<dyn EngineChild>,
    client: Arc<Aria2Client>,
    notify_rx: mpsc::UnboundedReceiver<Aria2Notification>,
    port: u16,
    secret: String,
}

async fn spawn_and_connect_with_retries(
    spawner: &dyn SidecarSpawner,
    data_dir: &std::path::Path,
    download_dir: &std::path::Path,
    speed_limit_kbps: Option<u32>,
    concurrency: Option<u32>,
) -> Result<Connected> {
    let host_pid = std::process::id();
    let mut last_err = None;
    for attempt in 0..SPAWN_ATTEMPTS {
        let aria_conf = match conf::write_conf(
            data_dir,
            download_dir,
            host_pid,
            speed_limit_kbps,
            concurrency,
        ) {
            Ok(c) => c,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let arg = format!("--conf-path={}", aria_conf.conf_path.display());
        let child = match spawner.spawn(&[arg], &[]).await {
            Ok(c) => c,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        match connect_and_health_check(aria_conf.port, aria_conf.secret.clone(), notify_tx).await {
            Ok(client) => {
                return Ok(Connected {
                    child,
                    client,
                    notify_rx,
                    port: aria_conf.port,
                    secret: aria_conf.secret,
                });
            }
            Err(e) => {
                let stdio = child.recent_stdio();
                child.kill().await;
                tracing::warn!(
                    attempt,
                    error = %e,
                    stdio = ?stdio,
                    "aria2 spawn/health-check attempt failed, retrying with a new port"
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| Aa4cError::Unavailable("aria2 spawn failed".into())))
}

async fn connect_and_health_check(
    port: u16,
    secret: String,
    notify_tx: mpsc::UnboundedSender<Aria2Notification>,
) -> Result<Arc<Aria2Client>> {
    let mut delay = HEALTH_CHECK_BASE_DELAY;
    let mut last_err = None;
    for _ in 0..HEALTH_CHECK_ATTEMPTS {
        match Aria2Client::connect(port, secret.clone(), notify_tx.clone()).await {
            Ok(client) => match client.call("aria2.getVersion", vec![]).await {
                Ok(_) => return Ok(client),
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    Err(last_err.unwrap_or_else(|| Aa4cError::Unavailable("aria2 health check failed".into())))
}

async fn reconnect_with_retries(
    port: u16,
    secret: String,
) -> Option<(Arc<Aria2Client>, mpsc::UnboundedReceiver<Aria2Notification>)> {
    let mut delay = Duration::from_millis(500);
    for _ in 0..RECONNECT_ATTEMPTS {
        tokio::time::sleep(delay).await;
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        if let Ok(client) = Aria2Client::connect(port, secret.clone(), notify_tx).await {
            if client.call("aria2.getVersion", vec![]).await.is_ok() {
                return Some((client, notify_rx));
            }
        }
        delay = (delay * 2).min(Duration::from_secs(8));
    }
    None
}

async fn actor_loop(
    store: Store,
    events: EventSender,
    mut connected: Connected,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
) {
    let mut poll = tokio::time::interval(RECONCILE_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = connected.client.closed() => {
                tracing::warn!("aria2 rpc connection lost, attempting to reconnect");
                match reconnect_with_retries(connected.port, connected.secret.clone()).await {
                    Some((client, notify_rx)) => {
                        connected.client = client;
                        connected.notify_rx = notify_rx;
                        tracing::info!("aria2 rpc reconnected");
                        reconcile(&store, &events, &connected.client).await;
                    }
                    None => {
                        tracing::error!(
                            "aria2 rpc reconnect exhausted; download capability unavailable for the rest of this session"
                        );
                        connected.child.kill().await;
                        return;
                    }
                }
            }
            note = connected.notify_rx.recv() => {
                // 通知只当"该重新对账了"的信号，不逐条精细处理——对账函数本身
                // 是幂等的（未变化的任务不会重复写库/重复广播），个人量级的任务
                // 列表下这点 RPC 开销可忽略，换来实现简单很多（DOWNLOAD_DESIGN.md
                // §3.2 原描述的逐条 tellStatus 收敛成这一种更粗但更简单的形态）。
                if note.is_some() {
                    reconcile(&store, &events, &connected.client).await;
                }
            }
            _ = poll.tick() => {
                reconcile(&store, &events, &connected.client).await;
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Cmd::Shutdown { reply }) => {
                        let _ = connected.client.call("aria2.shutdown", vec![]).await;
                        tokio::time::sleep(SHUTDOWN_GRACE).await;
                        connected.child.kill().await;
                        let _ = reply.send(());
                        return;
                    }
                    Some(other) => handle_command(&connected.client, &store, other).await,
                    None => {
                        // DownloadService 已销毁（全部 cmd_tx 已 drop）：静默清理退出，
                        // 不是错误路径，不需要日志。
                        connected.child.kill().await;
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_command(client: &Aria2Client, store: &Store, cmd: Cmd) {
    match cmd {
        Cmd::Add { url, reply } => {
            let result = add_download(client, store, url).await;
            let _ = reply.send(result);
        }
        Cmd::Pause { id, reply } => {
            let result = client
                .call("aria2.pause", vec![json!(id)])
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        Cmd::Resume { id, reply } => {
            let result = client
                .call("aria2.unpause", vec![json!(id)])
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        Cmd::Cancel { id, reply } => {
            // 活跃/等待态用 remove；已停止/出错态 remove 会报错，forceRemove 兜底——
            // 两次尝试都不关心谁成功了，落库状态才是最终事实来源。
            let _ = client.call("aria2.remove", vec![json!(id.clone())]).await;
            let _ = client
                .call("aria2.forceRemove", vec![json!(id.clone())])
                .await;
            let result = store
                .update_download_status(&id, DownloadStatus::Removed, None, None)
                .await;
            let _ = reply.send(result);
        }
        Cmd::ForgetCompleted { id, reply } => {
            // 跟 Cancel 不同：这里**要**如实回报 RPC 是否成功——调用方
            // （`clear_completed`）只在这个调用成功时才会删对应的 DB 行，不然
            // 引擎那边还认得这个任务，写库删掉记录只会在下次对账时被当孤儿
            // 补插回去。`aria2.remove`/`forceRemove` 是给活跃/等待态用的，对
            // 已停止（含 complete）的任务要用 `removeDownloadResult` 才能把它
            // 从 aria2 自己的 `tellStopped` 历史里摘掉。
            let result = client
                .call("aria2.removeDownloadResult", vec![json!(id)])
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        Cmd::Shutdown { .. } => unreachable!("Shutdown handled directly in actor_loop"),
    }
}

async fn add_download(client: &Aria2Client, store: &Store, url: String) -> Result<TaskId> {
    let result = client
        .call("aria2.addUri", vec![json!([url.clone()])])
        .await?;
    let gid = result
        .as_str()
        .ok_or_else(|| Aa4cError::Network("aria2.addUri did not return a gid".into()))?
        .to_string();
    store
        .insert_download(&gid, DownloadKind::Http, &url)
        .await?;
    Ok(gid)
}

/// 一条 aria2 任务快照（`tellActive`/`tellWaiting`/`tellStopped` 的元素）。
/// aria2 JSON-RPC 把数值字段编码成**字符串**（如 `"totalLength": "1024"`），
/// 这里统一解析成真实数值类型。
struct AriaInfo {
    status: String,
    total: u64,
    completed: u64,
    speed: u64,
    error_message: Option<String>,
    save_path: Option<String>,
    url: Option<String>,
}

fn parse_num(v: &Value) -> u64 {
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// aria2 `errorCode` → 人话（D3，DOWNLOAD_DESIGN.md §9）。对照表来自本机
/// `man aria2c`（Homebrew 装的真实 1.37.0 版本手册 EXIT STATUS 一节，不是网上
/// 抄的），只覆盖常见码，其余给通用兜底 + 附原始码方便排查。转译结果直接存进
/// `download_tasks.error`，不新增数据库列、不把原始 code 传到前端——错误人话
/// 化在这一层就做完，前端 `format.ts` 的 `errorText()` 转译的是另一层更粗粒度
/// 的 `Aa4cError::code()`，两者不是同一件事。
fn translate_aria_error(code: Option<u32>, raw_message: Option<&str>) -> Option<String> {
    let code = code.filter(|&c| c != 0)?;
    let text = match code {
        3 => "资源不存在（可能链接已失效）".to_string(),
        6 => "网络连接出了问题，请检查网络后重试".to_string(),
        9 => "磁盘空间不够了，清理一下磁盘或换个保存位置".to_string(),
        19 => "域名解析失败，请检查链接或网络".to_string(),
        22 => "服务器返回的响应有问题".to_string(),
        23 => "链接跳转次数过多，可能已失效".to_string(),
        24 => "需要登录或授权才能下载，暂不支持".to_string(),
        27 => "磁力链接格式不对，请检查后重新粘贴".to_string(),
        29 => "服务器暂时繁忙，请稍后重试".to_string(),
        _ => format!("下载出错（代码 {code}）"),
    };
    // raw_message 附在后面而不是丢弃——万一转译表覆盖不到具体细节，用户/未来
    // 排查还能看到 aria2 自己给的原始描述。
    match raw_message.filter(|s| !s.is_empty()) {
        Some(raw) => Some(format!("{text}（{raw}）")),
        None => Some(text),
    }
}

fn insert_aria_info(map: &mut HashMap<String, AriaInfo>, item: &Value) {
    let Some(gid) = item["gid"].as_str() else {
        return;
    };
    let error_code = item["errorCode"].as_str().and_then(|s| s.parse().ok());
    let raw_message = item["errorMessage"].as_str().filter(|s| !s.is_empty());
    let info = AriaInfo {
        status: item["status"].as_str().unwrap_or("error").to_string(),
        total: parse_num(&item["totalLength"]),
        completed: parse_num(&item["completedLength"]),
        speed: parse_num(&item["downloadSpeed"]),
        error_message: translate_aria_error(error_code, raw_message),
        save_path: item["files"][0]["path"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        url: item["files"][0]["uris"][0]["uri"]
            .as_str()
            .map(str::to_string),
    };
    map.insert(gid.to_string(), info);
}

async fn fetch_aria_snapshot(client: &Aria2Client) -> HashMap<String, AriaInfo> {
    let mut map = HashMap::new();
    if let Ok(Value::Array(items)) = client.call("aria2.tellActive", vec![]).await {
        for item in &items {
            insert_aria_info(&mut map, item);
        }
    }
    if let Ok(Value::Array(items)) = client
        .call("aria2.tellWaiting", vec![json!(0), json!(1000)])
        .await
    {
        for item in &items {
            insert_aria_info(&mut map, item);
        }
    }
    if let Ok(Value::Array(items)) = client
        .call("aria2.tellStopped", vec![json!(0), json!(1000)])
        .await
    {
        for item in &items {
            insert_aria_info(&mut map, item);
        }
    }
    map
}

fn broadcast_for_status(events: &EventSender, gid: &str, status: DownloadStatus, info: &AriaInfo) {
    let event = match status {
        DownloadStatus::Complete => CoreEvent::DownloadDone {
            task_id: gid.to_string(),
            save_path: info.save_path.clone().unwrap_or_default(),
        },
        DownloadStatus::Error => CoreEvent::DownloadFailed {
            task_id: gid.to_string(),
            error: info
                .error_message
                .clone()
                .unwrap_or_else(|| "unknown error".into()),
        },
        _ => CoreEvent::DownloadProgress {
            task_id: gid.to_string(),
            downloaded_bytes: info.completed,
            total_bytes: info.total,
            speed_bps: info.speed,
            // aria2（HTTP/HTTPS/FTP）没有做种数/peer 数/分享率这个概念——
            // Transmission（D2）的等价路径才会填这三个字段。
            seeders: None,
            peers: None,
            ratio: None,
        },
    };
    let _ = events.send(event);
}

/// 启动时 / WS 重连后 / 收到通知 / 每个轮询节拍都跑这一套（DOWNLOAD_DESIGN.md
/// §3.4）：两边都有 → 以 aria2 为准刷新；表里未完态但 aria2 没有 → 标失败
/// （孤儿记录，同 `restart_marks_stale_tasks_failed` 对 `transfer_tasks` 的先例）；
/// aria2 有表没有 → 补插一行（补齐能拿到的 url，写库前恰好崩溃的窗口）。
/// 幂等：只在状态/进度真的变化时才写库、广播——避免完成/失败态被反复重放。
async fn reconcile(store: &Store, events: &EventSender, client: &Aria2Client) {
    let aria_map = fetch_aria_snapshot(client).await;
    let stored = store.list_downloads().await.unwrap_or_default();
    let mut remaining: HashMap<String, aa4c_types::DownloadTask> =
        stored.into_iter().map(|t| (t.id.clone(), t)).collect();

    for (gid, info) in &aria_map {
        let status: DownloadStatus = info.status.parse().unwrap_or(DownloadStatus::Error);
        match remaining.remove(gid) {
            None => {
                if store
                    .insert_download(gid, DownloadKind::Http, info.url.as_deref().unwrap_or(""))
                    .await
                    .is_ok()
                {
                    let _ = store
                        .update_download_status(
                            gid,
                            status,
                            info.error_message.as_deref(),
                            info.save_path.as_deref(),
                        )
                        .await;
                    let _ = store
                        .update_download_progress(gid, info.completed, info.total)
                        .await;
                    broadcast_for_status(events, gid, status, info);
                }
            }
            Some(existing) => {
                let changed = existing.status != status
                    || existing.downloaded_bytes != info.completed
                    || existing.total_bytes != info.total;
                if changed {
                    let _ = store
                        .update_download_status(
                            gid,
                            status,
                            info.error_message.as_deref(),
                            info.save_path.as_deref(),
                        )
                        .await;
                    let _ = store
                        .update_download_progress(gid, info.completed, info.total)
                        .await;
                }
                if existing.status != status {
                    broadcast_for_status(events, gid, status, info);
                } else if status == DownloadStatus::Active {
                    // 状态没变但仍在下载：进度条的唯一数据源，照样广播。
                    let _ = events.send(CoreEvent::DownloadProgress {
                        task_id: gid.clone(),
                        downloaded_bytes: info.completed,
                        total_bytes: info.total,
                        speed_bps: info.speed,
                        seeders: None,
                        peers: None,
                        ratio: None,
                    });
                }
            }
        }
    }

    // aria2 这次快照里没出现、但表里仍是未完态的：孤儿记录，标失败。
    for (gid, task) in remaining {
        if matches!(
            task.status,
            DownloadStatus::Active | DownloadStatus::Waiting | DownloadStatus::Paused
        ) {
            const MSG: &str = "应用重启后任务已丢失，请重新添加";
            let _ = store
                .update_download_status(&gid, DownloadStatus::Error, Some(MSG), None)
                .await;
            let _ = events.send(CoreEvent::DownloadFailed {
                task_id: gid,
                error: MSG.into(),
            });
        }
    }
}

// ---------------------------------------------------------------------
// Transmission（BT/Magnet，D2）actor——独立于上面 aria2 那一套，鉴权模型
// （HTTP Basic + session header）、传输（纯请求-响应，无事件推送）都完全
// 不同，见 `transmission_rpc.rs` 顶部文档。
// ---------------------------------------------------------------------

struct BtConnected {
    proc: TransmissionProcess,
    client: Arc<TransmissionClient>,
    port: u16,
}

async fn spawn_and_connect_bt_with_retries(
    spawner: &dyn SidecarSpawner,
    data_dir: &std::path::Path,
    download_dir: &std::path::Path,
    speed_limit_kbps: Option<u32>,
    concurrency: Option<u32>,
    ratio_limit: Option<f64>,
    idle_seeding_limit_minutes: Option<u32>,
) -> Result<BtConnected> {
    let mut last_err = None;
    for attempt in 0..SPAWN_ATTEMPTS {
        let proc = match TransmissionProcess::spawn(
            spawner,
            data_dir,
            download_dir,
            speed_limit_kbps,
            concurrency,
            ratio_limit,
            idle_seeding_limit_minutes,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let client = Arc::new(TransmissionClient::new(
            proc.port,
            &proc.username,
            &proc.password,
        ));
        match bt_health_check(&client).await {
            Ok(()) => {
                let port = proc.port;
                return Ok(BtConnected { proc, client, port });
            }
            Err(e) => {
                let stdio = proc.recent_stdio();
                proc.kill().await;
                tracing::warn!(
                    attempt,
                    error = %e,
                    stdio = ?stdio,
                    "transmission spawn/health-check attempt failed, retrying with a new port"
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| Aa4cError::Unavailable("transmission spawn failed".into())))
}

async fn bt_health_check(client: &TransmissionClient) -> Result<()> {
    let mut delay = HEALTH_CHECK_BASE_DELAY;
    let mut last_err = None;
    for _ in 0..HEALTH_CHECK_ATTEMPTS {
        match client.call("session-get", json!({})).await {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    Err(last_err
        .unwrap_or_else(|| Aa4cError::Unavailable("transmission health check failed".into())))
}

async fn bt_actor_loop(
    store: Store,
    events: EventSender,
    connected: BtConnected,
    mut cmd_rx: mpsc::UnboundedReceiver<BtCmd>,
) {
    let mut poll = tokio::time::interval(RECONCILE_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut consecutive_failures = 0u32;

    loop {
        tokio::select! {
            _ = poll.tick() => {
                match bt_reconcile(&store, &events, &connected.client).await {
                    Ok(()) => consecutive_failures = 0,
                    Err(e) => {
                        consecutive_failures += 1;
                        tracing::warn!(
                            error = %e,
                            consecutive_failures,
                            "transmission rpc poll failed"
                        );
                        if consecutive_failures >= BT_FAILURE_THRESHOLD {
                            tracing::error!(
                                "transmission rpc unreachable after {BT_FAILURE_THRESHOLD} consecutive attempts; \
                                 BT capability unavailable for the rest of this session"
                            );
                            connected.proc.kill().await;
                            return;
                        }
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(BtCmd::Shutdown { reply }) => {
                        let _ = connected.client.call("session-close", json!({})).await;
                        tokio::time::sleep(SHUTDOWN_GRACE).await;
                        connected.proc.kill().await;
                        let _ = reply.send(());
                        return;
                    }
                    Some(other) => handle_bt_command(&connected.client, &store, other).await,
                    None => {
                        // DownloadService 已销毁：静默清理退出，不是错误路径。
                        connected.proc.kill().await;
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_bt_command(client: &TransmissionClient, store: &Store, cmd: BtCmd) {
    match cmd {
        BtCmd::Add { magnet, reply } => {
            let result = add_bt_download(client, store, magnet).await;
            let _ = reply.send(result);
        }
        BtCmd::Pause { id, reply } => {
            let result = client
                .call("torrent-stop", json!({ "ids": [id] }))
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        BtCmd::Resume { id, reply } => {
            let result = client
                .call("torrent-start", json!({ "ids": [id] }))
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        BtCmd::Cancel { id, reply } => {
            // 同 aria2 侧的取舍：不关心 RPC 调用本身成不成功，落库状态才是
            // 最终事实来源（引擎可能已经不在了，取消动作也该在本地生效）。
            let _ = client
                .call(
                    "torrent-remove",
                    json!({ "ids": [id.clone()], "delete-local-data": false }),
                )
                .await;
            let result = store
                .update_download_status(&id, DownloadStatus::Removed, None, None)
                .await;
            let _ = reply.send(result);
        }
        BtCmd::ForgetCompleted { id, reply } => {
            // 跟 Cancel 不同：这里**要**如实回报是否成功——`torrent-remove` 对
            // 任意状态的种子都适用（不区分活跃/完成，跟 aria2 需要换一个 RPC
            // 方法不同），但调用方（`clear_completed`）要靠这个结果决定是否
            // 删对应的 DB 行，不能像 Cancel 那样"不管成不成功都当作已生效"。
            let result = client
                .call(
                    "torrent-remove",
                    json!({ "ids": [id], "delete-local-data": false }),
                )
                .await
                .map(|_| ());
            let _ = reply.send(result);
        }
        BtCmd::Shutdown { .. } => unreachable!("Shutdown handled directly in bt_actor_loop"),
    }
}

async fn add_bt_download(
    client: &TransmissionClient,
    store: &Store,
    magnet: String,
) -> Result<TaskId> {
    let result = client
        .call(
            "torrent-add",
            json!({ "filename": magnet, "paused": false }),
        )
        .await?;
    // 已经存在的 magnet 会走 `torrent-duplicate` 分支而不是 `torrent-added`——
    // 两种情况都要接受（用户重复粘贴同一个链接不该报错，同 aria2 侧幂等取舍）。
    let hash = result["torrent-added"]["hashString"]
        .as_str()
        .or_else(|| result["torrent-duplicate"]["hashString"].as_str())
        .ok_or_else(|| {
            Aa4cError::Network("transmission torrent-add did not return a hashString".into())
        })?
        .to_string();
    store
        .insert_download(&hash, DownloadKind::Bt, &magnet)
        .await?;
    Ok(hash)
}

/// 一条 Transmission 任务快照（`torrent-get` 的元素）。字段名沿用 Transmission
/// RPC 原生命名（camelCase 是它自己的约定，不是我们这边套的）。
struct BtInfo {
    status: DownloadStatus,
    total: u64,
    completed: u64,
    speed: u64,
    error_message: Option<String>,
    save_path: Option<String>,
    seeders: u32,
    peers: u32,
    ratio: f64,
}

/// Transmission 数字状态码 → 统一六态（DOWNLOAD_DESIGN.md §3.6.4）：
/// `percentDone` 到 1.0 就算完成，不管这时候的 `status` 是"停止"还是"做种中"
/// ——完成后继续做种是 BT 的常规行为，不是"还没下完"。
fn map_bt_status(status: i64, percent_done: f64, error: i64) -> DownloadStatus {
    if percent_done >= 1.0 {
        return DownloadStatus::Complete;
    }
    if error != 0 {
        return DownloadStatus::Error;
    }
    match status {
        4 | 6 => DownloadStatus::Active,          // downloading / seeding
        1 | 2 | 3 | 5 => DownloadStatus::Waiting, // *-wait / checking
        0 => DownloadStatus::Paused,              // stopped
        _ => DownloadStatus::Error,
    }
}

const BT_TORRENT_GET_FIELDS: &[&str] = &[
    "hashString",
    "name",
    "status",
    "percentDone",
    "sizeWhenDone",
    "leftUntilDone",
    "rateDownload",
    "peersConnected",
    "peersSendingToUs",
    "uploadRatio",
    "error",
    "errorString",
    "downloadDir",
];

/// 拉取全量快照。**故意不吞错误**（不同于 aria2 侧的 `fetch_aria_snapshot`）：
/// Transmission 没有持久连接可以监听"断开"，这次 RPC 调用本身能不能成功就是
/// 唯一的存活信号——调用方（`bt_actor_loop`）靠这个 `Result` 数连续失败次数，
/// 判断 daemon 是不是已经崩了；调用失败时绝不能把"引擎不可达"误判成"引擎
/// 说现在没有任何任务"，那会把还在运行的任务错误地标成孤儿。
async fn fetch_bt_snapshot(client: &TransmissionClient) -> Result<HashMap<String, BtInfo>> {
    let result = client
        .call("torrent-get", json!({ "fields": BT_TORRENT_GET_FIELDS }))
        .await?;
    let mut map = HashMap::new();
    let Some(torrents) = result["torrents"].as_array() else {
        return Ok(map);
    };
    for item in torrents {
        let Some(hash) = item["hashString"].as_str() else {
            continue;
        };
        let status_code = item["status"].as_i64().unwrap_or(0);
        let percent_done = item["percentDone"].as_f64().unwrap_or(0.0);
        let error_code = item["error"].as_i64().unwrap_or(0);
        let size_when_done = item["sizeWhenDone"].as_u64().unwrap_or(0);
        let left_until_done = item["leftUntilDone"].as_u64().unwrap_or(0);
        let completed = size_when_done.saturating_sub(left_until_done);
        let error_message = item["errorString"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let save_path = item["downloadDir"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        map.insert(
            hash.to_string(),
            BtInfo {
                status: map_bt_status(status_code, percent_done, error_code),
                total: size_when_done,
                completed,
                speed: item["rateDownload"].as_u64().unwrap_or(0),
                error_message,
                save_path,
                seeders: item["peersSendingToUs"].as_u64().unwrap_or(0) as u32,
                peers: item["peersConnected"].as_u64().unwrap_or(0) as u32,
                ratio: item["uploadRatio"].as_f64().unwrap_or(0.0),
            },
        );
    }
    Ok(map)
}

fn broadcast_bt_progress(events: &EventSender, hash: &str, status: DownloadStatus, info: &BtInfo) {
    let event = match status {
        DownloadStatus::Complete => CoreEvent::DownloadDone {
            task_id: hash.to_string(),
            save_path: info.save_path.clone().unwrap_or_default(),
        },
        DownloadStatus::Error => CoreEvent::DownloadFailed {
            task_id: hash.to_string(),
            error: info
                .error_message
                .clone()
                .unwrap_or_else(|| "unknown error".into()),
        },
        _ => CoreEvent::DownloadProgress {
            task_id: hash.to_string(),
            downloaded_bytes: info.completed,
            total_bytes: info.total,
            speed_bps: info.speed,
            seeders: Some(info.seeders),
            peers: Some(info.peers),
            ratio: Some(info.ratio),
        },
    };
    let _ = events.send(event);
}

/// 结构完全照抄 aria2 侧的 `reconcile()`（DOWNLOAD_DESIGN.md §3.4 的对账逻辑对
/// 两个引擎是同一套：启动时/轮询时都跑，两边都有则刷新，表里未完但引擎没有则
/// 标孤儿失败，引擎有表没有则补插）——**关键差异**是这个函数会把 RPC 调用
/// 失败原样传播给调用方（见 `fetch_bt_snapshot` 的文档），不像 aria2 那边可以
/// 依赖 WS 连接断开事件区分"真的没任务"和"RPC 打不通"。
async fn bt_reconcile(
    store: &Store,
    events: &EventSender,
    client: &TransmissionClient,
) -> Result<()> {
    let bt_map = fetch_bt_snapshot(client).await?;
    let stored = store.list_downloads().await.unwrap_or_default();
    let mut remaining: HashMap<String, aa4c_types::DownloadTask> = stored
        .into_iter()
        .filter(|t| t.kind == DownloadKind::Bt)
        .map(|t| (t.id.clone(), t))
        .collect();

    for (hash, info) in &bt_map {
        match remaining.remove(hash) {
            None => {
                // 引擎有、表里没有：补插一行（`add_bt_download` 写库前恰好
                // 崩溃的窗口，或者用户直接用别的客户端往这个 daemon 加了任务）。
                // 补不上原始 magnet，退化用 hash 本身占位——不影响任务可操作性。
                if store
                    .insert_download(
                        hash,
                        DownloadKind::Bt,
                        &format!("magnet:?xt=urn:btih:{hash}"),
                    )
                    .await
                    .is_ok()
                {
                    let _ = store
                        .update_download_status(
                            hash,
                            info.status,
                            info.error_message.as_deref(),
                            info.save_path.as_deref(),
                        )
                        .await;
                    let _ = store
                        .update_download_progress(hash, info.completed, info.total)
                        .await;
                    broadcast_bt_progress(events, hash, info.status, info);
                }
            }
            Some(existing) => {
                let changed = existing.status != info.status
                    || existing.downloaded_bytes != info.completed
                    || existing.total_bytes != info.total;
                if changed {
                    let _ = store
                        .update_download_status(
                            hash,
                            info.status,
                            info.error_message.as_deref(),
                            info.save_path.as_deref(),
                        )
                        .await;
                    let _ = store
                        .update_download_progress(hash, info.completed, info.total)
                        .await;
                }
                // 状态变了必广播；状态没变但仍在下载中的，进度条的唯一数据源，
                // 照样广播（同 aria2 侧 `reconcile()` 的取舍）。
                if existing.status != info.status || info.status == DownloadStatus::Active {
                    broadcast_bt_progress(events, hash, info.status, info);
                }
            }
        }
    }

    // Transmission 这次快照里没出现、但表里仍是未完态的 BT 任务：孤儿记录，标失败。
    for (hash, task) in remaining {
        if matches!(
            task.status,
            DownloadStatus::Active | DownloadStatus::Waiting | DownloadStatus::Paused
        ) {
            const MSG: &str = "应用重启后任务已丢失，请重新添加";
            let _ = store
                .update_download_status(&hash, DownloadStatus::Error, Some(MSG), None)
                .await;
            let _ = events.send(CoreEvent::DownloadFailed {
                task_id: hash,
                error: MSG.into(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_aria_error_covers_common_codes() {
        assert_eq!(
            translate_aria_error(Some(3), None).unwrap(),
            "资源不存在（可能链接已失效）"
        );
        assert_eq!(
            translate_aria_error(Some(9), None).unwrap(),
            "磁盘空间不够了，清理一下磁盘或换个保存位置"
        );
        assert_eq!(
            translate_aria_error(Some(27), None).unwrap(),
            "磁力链接格式不对，请检查后重新粘贴"
        );
    }

    #[test]
    fn translate_aria_error_falls_back_for_unknown_codes() {
        assert_eq!(
            translate_aria_error(Some(99), None).unwrap(),
            "下载出错（代码 99）"
        );
    }

    #[test]
    fn translate_aria_error_appends_raw_message_when_present() {
        let text = translate_aria_error(Some(6), Some("Connection refused")).unwrap();
        assert!(text.starts_with("网络连接出了问题"));
        assert!(text.contains("Connection refused"));
    }

    #[test]
    fn translate_aria_error_none_when_code_is_zero_or_absent() {
        assert!(translate_aria_error(Some(0), Some("ignored")).is_none());
        assert!(translate_aria_error(None, None).is_none());
    }

    #[test]
    fn bt_task_id_routes_by_length() {
        // aria2 GID：16 位十六进制；BT infohash：40 位十六进制（SHA-1）。
        assert!(!is_bt_task_id("0123456789abcdef"));
        assert!(is_bt_task_id(&"a".repeat(40)));
    }
}
