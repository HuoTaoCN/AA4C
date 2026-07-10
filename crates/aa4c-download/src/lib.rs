//! 下载中心服务（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md）。
//!
//! 独立于设备身份/配对——下载没有"对端设备"概念。对上层（`aa4c-core`）只暴露
//! 任务模型（`add`/`pause`/`resume`/`cancel`/`list`），引擎细节（aria2 子进程 +
//! JSON-RPC）全部封装在这个 crate 内部。
//!
//! 内部是一个单线程 actor：一个后台任务独占持有"当前连接"状态（子进程句柄 +
//! RPC 客户端），公开方法通过 channel 发命令、等回复——避免用锁去保护"连接可能
//! 随时因为重连而整体替换"这种状态，也让"服务当前不可用"有一个天然、单一的
//! 判定点（channel 发送失败 = actor 已退出 = 不可用）。

// D1 上线时是 `forbid`——D2 接孤儿进程防护需要直接调用 Win32 API（Job Object）
// 与 Linux `prctl`，两处都是必需的 unsafe FFI，`forbid` 连局部 `#[allow]` 都不
// 认，收窄成 `deny` + 只在 `orphan_guard` 模块里 `#[allow(unsafe_code)]`，其余
// 代码仍然维持"unsafe 默认不许"的原则不变。
#![deny(unsafe_code)]

mod conf;
mod orphan_guard;
mod rpc;
mod spawner;
mod transmission_conf;
mod transmission_process;
mod util;

pub use rpc::{Aria2Client, Aria2Notification};
pub use spawner::{EngineChild, KillFuture, ProcessSpawner, SidecarSpawner, SpawnFuture};
pub use transmission_process::TransmissionProcess;

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
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// 下载中心服务句柄。`cmd_tx` 为 `None`（引擎启动失败）或后台 actor 已退出
/// （重连耗尽/已 shutdown）时，全部操作性方法统一返回 `Aa4cError::Unavailable`。
pub struct DownloadService {
    store: Store,
    cmd_tx: Option<mpsc::UnboundedSender<Cmd>>,
}

impl DownloadService {
    /// 启动：拉起 aria2c、健康检查、启动时对账、起后台 actor。**启动失败不返回
    /// `Err`**——下载能力整体降级不可用，但不阻塞调用方（同 QUIC 端点等既有
    /// 可选能力的一贯降级设计，DOWNLOAD_DESIGN.md §3.1）。
    pub async fn start(
        spawner: Arc<dyn SidecarSpawner>,
        store: Store,
        events: EventSender,
        data_dir: PathBuf,
        download_dir: PathBuf,
    ) -> Arc<Self> {
        match spawn_and_connect_with_retries(spawner.as_ref(), &data_dir, &download_dir).await {
            Ok(connected) => {
                reconcile(&store, &events, &connected.client).await;
                tracing::info!(port = connected.port, "download engine (aria2c) connected");
                let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
                tokio::spawn(actor_loop(store.clone(), events, connected, cmd_rx));
                Arc::new(Self {
                    store,
                    cmd_tx: Some(cmd_tx),
                })
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "download engine unavailable at startup; download capability disabled for this session"
                );
                Arc::new(Self {
                    store,
                    cmd_tx: None,
                })
            }
        }
    }

    fn unavailable() -> Aa4cError {
        Aa4cError::Unavailable("download engine not available".into())
    }

    /// 新建一条下载任务（D1 只接受 HTTP/HTTPS/FTP 直链）。
    pub async fn add(&self, url: String) -> Result<TaskId> {
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Add { url, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn pause(&self, id: TaskId) -> Result<()> {
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Pause { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn resume(&self, id: TaskId) -> Result<()> {
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Resume { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    pub async fn cancel(&self, id: TaskId) -> Result<()> {
        let tx = self.cmd_tx.as_ref().ok_or_else(Self::unavailable)?;
        let (reply, rx) = oneshot::channel();
        tx.send(Cmd::Cancel { id, reply })
            .map_err(|_| Self::unavailable())?;
        rx.await.map_err(|_| Self::unavailable())?
    }

    /// 按创建时间倒序列出全部下载任务。数据库读取，不经过 actor（列表展示不需要
    /// 强一致于"引擎当前是否在线"，引擎不可用时仍应能看到历史记录）。
    pub async fn list(&self) -> Result<Vec<aa4c_types::DownloadTask>> {
        self.store.list_downloads().await
    }

    /// 优雅关闭：`aria2.shutdown`（触发一次 session 保存）→ 宽限期 → 强杀。
    /// 引擎本就不可用时是no-op。
    pub async fn shutdown(&self) {
        let Some(tx) = &self.cmd_tx else { return };
        let (reply, rx) = oneshot::channel();
        if tx.send(Cmd::Shutdown { reply }).is_ok() {
            let _ = rx.await;
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
) -> Result<Connected> {
    let host_pid = std::process::id();
    let mut last_err = None;
    for attempt in 0..SPAWN_ATTEMPTS {
        let aria_conf = match conf::write_conf(data_dir, download_dir, host_pid) {
            Ok(c) => c,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let arg = format!("--conf-path={}", aria_conf.conf_path.display());
        let child = match spawner.spawn(&[arg]).await {
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

fn insert_aria_info(map: &mut HashMap<String, AriaInfo>, item: &Value) {
    let Some(gid) = item["gid"].as_str() else {
        return;
    };
    let info = AriaInfo {
        status: item["status"].as_str().unwrap_or("error").to_string(),
        total: parse_num(&item["totalLength"]),
        completed: parse_num(&item["completedLength"]),
        speed: parse_num(&item["downloadSpeed"]),
        error_message: item["errorMessage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
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
