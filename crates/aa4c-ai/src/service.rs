//! `AiService`：懒启动 + 空闲自停的双槽位（对话/嵌入）管理（ARCHIVE_DESIGN.md
//! §3.3）。与 `DownloadService`（`aa4c-download`）的关键差异——那边是"轻量常驻，
//! 一个引擎两个 actor 各管一条长连接"；这边**绝不常驻**（LLM 引擎吃内存，
//! 4B Q4 模型 ≈3GB RAM），每个槽位只在真正被用到时才拉起进程，空闲一段时间
//! 后自动退出释放内存。两个槽位各自独立进程、独立模型，一个槽位不可用
//! 不影响另一个（同 D1/D2 aria2/Transmission 互不影响的既有先例）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use aa4c_engine::SidecarSpawner;
use aa4c_types::{Aa4cError, AiEngineStatus, AiSlot, AiSlotStatus, CoreEvent, Result};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::client::LlamaClient;
use crate::process::{LlamaProcess, SlotKind};

/// 两个槽位各自的静态配置（模型路径缺失 = 那个槽位整体 `Unavailable`，同
/// 下载能力缺失的既有降级语义）+ 共用的空闲超时。
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub chat_model: Option<PathBuf>,
    pub embedding_model: Option<PathBuf>,
    /// 默认 10 分钟（ARCHIVE_DESIGN.md §3.3）。
    pub idle_timeout: Duration,
    /// PID 文件存放目录（macOS 孤儿进程防护兜底用，`aa4c-engine::OrphanPidfile`
    /// 先例）——两个槽位各用一个文件名，互不覆盖。
    pub state_dir: PathBuf,
}

struct RunningSlot {
    proc: LlamaProcess,
    /// 用 `Arc<StdMutex<_>>` 而不是普通字段：流式请求在拿到 `LlamaClient`
    /// 之后会主动释放这个槽位的大锁（见 `chat_completion_stream` 的文档），
    /// 但后台转发任务仍需要在每收到一个 token 时"续命"，必须能在不重新
    /// 竞争大锁的前提下更新——这是唯一需要脱离大锁单独更新的字段，其余状态
    /// 全部在持锁区间内变化，不需要这层间接。
    last_used: Arc<StdMutex<Instant>>,
}

struct Slot {
    model_path: Option<PathBuf>,
    running: Option<RunningSlot>,
}

pub struct AiService {
    spawner: Arc<dyn SidecarSpawner>,
    state_dir: PathBuf,
    idle_timeout: Duration,
    events: broadcast::Sender<CoreEvent>,
    chat: Mutex<Slot>,
    embedding: Mutex<Slot>,
}

impl AiService {
    /// 不做任何拉起动作——两个槽位都是懒启动，`start()` 只登记配置、起一个
    /// 后台巡查任务（周期见 `janitor_loop`）。巡查任务持有 `Weak` 引用，
    /// `AiService` 被整体 drop 时它会在下一次醒来时自然退出，不需要额外的
    /// 关闭信号（同 Rust 里"用 Weak 让后台任务自行终止"的常见写法，比额外
    /// 起一个 `oneshot` 关闭 channel 更省心，这里两个槽位共用一个巡查任务，
    /// 关闭时机也天然一致）。
    pub fn start(
        spawner: Arc<dyn SidecarSpawner>,
        config: AiConfig,
        events: broadcast::Sender<CoreEvent>,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            spawner,
            state_dir: config.state_dir,
            idle_timeout: config.idle_timeout,
            events,
            chat: Mutex::new(Slot {
                model_path: config.chat_model,
                running: None,
            }),
            embedding: Mutex::new(Slot {
                model_path: config.embedding_model,
                running: None,
            }),
        });
        let tick = service
            .idle_timeout
            .min(Duration::from_secs(30))
            .max(Duration::from_secs(1));
        tokio::spawn(janitor_loop(Arc::downgrade(&service), tick));
        service
    }

    /// 非流式聊天补全。整个请求期间持续给槽位"续命"，理由见 [`keep_alive_while`]。
    pub async fn chat_completion(&self, request: Value) -> Result<Value> {
        let (client, last_used) = self.ensure_running(SlotKind::Chat).await?;
        keep_alive_while(&last_used, client.chat_completion(request)).await
    }

    /// 批量嵌入。同 [`Self::chat_completion`]：一批几百个 chunk 在 CPU 上算完
    /// 完全可能超过 `idle_timeout`，所以也要全程续命。
    pub async fn embeddings(&self, request: Value) -> Result<Value> {
        let (client, last_used) = self.ensure_running(SlotKind::Embedding).await?;
        keep_alive_while(&last_used, client.embeddings(request)).await
    }

    /// 流式聊天补全：内部再包一层转发 channel，让后台转发任务在每收到一个
    /// token 时都给这个槽位"续命"（`last_used` 刷新）——不这样做的话，一个
    /// 耗时超过 `idle_timeout` 的长回复会在生成过程中被巡查任务当成"空闲"
    /// 提前杀掉。转发任务与巡查任务都只碰 `Arc<StdMutex<Instant>>`，不重新
    /// 竞争槽位大锁，两者不会互相阻塞。
    pub async fn chat_completion_stream(
        &self,
        request: Value,
    ) -> Result<mpsc::UnboundedReceiver<Result<Value>>> {
        let (client, last_used) = self.ensure_running(SlotKind::Chat).await?;
        let mut inner_rx = client.chat_completion_stream(request)?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(item) = inner_rx.recv().await {
                *last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                if tx.send(item).is_err() {
                    break; // 接收端已经不要了（调用方丢弃了 receiver），提前收手。
                }
            }
        });
        Ok(rx)
    }

    /// 停止两个槽位（Core 整体关闭时调用）——幂等，已停的槽位不会重复触发
    /// `Stopped` 事件。
    pub async fn shutdown(&self) {
        self.stop_if_running(SlotKind::Chat).await;
        self.stop_if_running(SlotKind::Embedding).await;
    }

    /// 查询一个槽位当前的静态快照（供 `get_ai_status` 这类一次性查询用，不
    /// 经事件总线）——`configured` 是"有没有配模型文件"，`running` 是"进程
    /// 现在是不是真的活着"，两者独立：配了模型但还没被用过（懒启动）时
    /// `configured=true, running=false` 是完全正常的状态，不是错误。
    pub async fn status(&self, kind: SlotKind) -> AiSlotStatus {
        let slot = self.slot_mutex(kind).lock().await;
        AiSlotStatus {
            configured: slot.model_path.is_some(),
            running: slot.running.is_some(),
        }
    }

    /// 确保槽位已就绪，返回一个可独立使用、不再占着槽位大锁的 `LlamaClient`
    /// 克隆（`LlamaClient` 只有 `port`+`auth_header` 两个廉价字段，克隆开销
    /// 可忽略）+ 这个槽位的 `last_used` 句柄。**为什么要在这里就放手大锁**：
    /// 一次推理请求可能要跑几十秒到几分钟（CPU 慢是常态），如果让大锁贯穿
    /// 整个请求期间，两个并发请求会互相排队、巡查任务也进不来——用一个共享
    /// 的 `last_used` 句柄替代"全程持锁"，换来的是一个有界的、可接受的
    /// 竞态窗口（详见下方 `chat_completion`/`embeddings` 内联说明），不是
    /// 免费的，但比"长请求互相阻塞"这个确定会发生的问题更值得接受。
    async fn ensure_running(
        &self,
        kind: SlotKind,
    ) -> Result<(LlamaClient, Arc<StdMutex<Instant>>)> {
        let mut slot = self.slot_mutex(kind).lock().await;

        if slot.running.is_none() {
            let model_path = slot.model_path.clone().ok_or_else(|| {
                Aa4cError::Unavailable(format!("{} model not configured", slot_label(kind)))
            })?;
            self.emit(kind, AiEngineStatus::Starting, None);
            let pidfile_path = self
                .state_dir
                .join(format!("llama-{}.pid", slot_label(kind)));
            match LlamaProcess::spawn(self.spawner.as_ref(), &model_path, kind, pidfile_path).await
            {
                Ok(proc) => {
                    self.emit(kind, AiEngineStatus::Ready, None);
                    slot.running = Some(RunningSlot {
                        proc,
                        last_used: Arc::new(StdMutex::new(Instant::now())),
                    });
                }
                Err(e) => {
                    self.emit(kind, AiEngineStatus::Unavailable, Some(e.to_string()));
                    return Err(e);
                }
            }
        }

        let running = slot.running.as_ref().expect("just ensured Some above");
        *running.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        Ok((running.proc.client().clone(), running.last_used.clone()))
    }

    /// 用户在设置页换了模型文件时调用（`Core::update_settings` 检测到
    /// `ai_chat_model`/`ai_embedding_model` 变化后转发）：更新槽位记住的模型
    /// 路径，若该槽位当前正跑着（旧模型），顺手停掉——不这样做的话，正在跑的
    /// 进程会继续伺服旧模型直到自然空闲超时，用户选了新模型却感知不到已经
    /// 生效。下一次请求会用新路径懒启动，不需要重启整个应用。
    pub async fn set_model(&self, kind: SlotKind, model_path: Option<PathBuf>) {
        let mut slot = self.slot_mutex(kind).lock().await;
        slot.model_path = model_path;
        if let Some(running) = slot.running.take() {
            running.proc.shutdown().await;
            self.emit(kind, AiEngineStatus::Stopped, None);
        }
    }

    async fn stop_if_running(&self, kind: SlotKind) {
        let mut slot = self.slot_mutex(kind).lock().await;
        if let Some(running) = slot.running.take() {
            running.proc.shutdown().await;
            self.emit(kind, AiEngineStatus::Stopped, None);
        }
    }

    fn slot_mutex(&self, kind: SlotKind) -> &Mutex<Slot> {
        match kind {
            SlotKind::Chat => &self.chat,
            SlotKind::Embedding => &self.embedding,
        }
    }

    fn emit(&self, kind: SlotKind, status: AiEngineStatus, error: Option<String>) {
        let slot = match kind {
            SlotKind::Chat => AiSlot::Chat,
            SlotKind::Embedding => AiSlot::Embedding,
        };
        // 没有订阅者时 `send` 返回 Err（同 Core 事件总线的既有先例），不是
        // 需要处理的错误——UI 没打开的时候丢事件是预期行为。
        let _ = self.events.send(CoreEvent::AiEngineState {
            slot,
            status,
            error,
        });
    }
}

fn slot_label(kind: SlotKind) -> &'static str {
    match kind {
        SlotKind::Chat => "chat",
        SlotKind::Embedding => "embedding",
    }
}

/// 周期性检查两个槽位是否已经空闲超过 `idle_timeout`，超过就优雅退出释放
/// 内存。`service` 是 `Weak` 引用——`AiService` 被整体 drop 后 `upgrade()`
/// 返回 `None`，这个循环自然结束，不需要额外的关闭信号。
/// 请求在飞的整个期间，每 [`KEEP_ALIVE_TICK`] 刷新一次这个槽位的 `last_used`。
///
/// **不这样做会被巡查任务半路杀掉。** `ensure_running` 拿到 `LlamaClient` 之后就
/// 主动放开了槽位大锁（一次推理动辄几十秒到几分钟，全程持锁会让并发请求互相排队），
/// 巡查任务因此能在请求进行中拿到锁并检查 `last_used`——而 `last_used` 上一次更新
/// 还停在**请求开始之前**。于是任何耗时超过 `idle_timeout` 的单次请求都会被判成
/// "空闲"，进程被关掉，正在等的那个请求收到 `connection closed before http headers
/// completed`。
///
/// 流式路径（[`AiService::chat_completion_stream`]）从一开始就有这层保护——每收到一个
/// token 就续一次命——非流式的两条却漏了，直到 2026-09-11 CI 的 macOS 腿把它暴露出来：
/// 共享 runner 上同一个测试二进制里几个用例各自起 llama-server 互相抢核，一次本机
/// 0.75s 的生成拖过了那条用例设的 30s `idle_timeout`。生产上同样成立——慢机器、大模型、
/// 长回复，任何一条都够。
///
/// 代价是"请求卡死则进程不会被回收"，这与流式路径的既有取舍完全一致：判断依据是
/// "这个槽位还有人在用吗"，不是"这个槽位有没有在产出"。
async fn keep_alive_while<T>(
    last_used: &Arc<StdMutex<Instant>>,
    fut: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(fut);
    *last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    let mut ticker = tokio::time::interval(KEEP_ALIVE_TICK);
    ticker.tick().await; // interval 的第一拍立即完成，先吃掉它
    loop {
        tokio::select! {
            out = &mut fut => {
                *last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                return out;
            }
            _ = ticker.tick() => {
                *last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            }
        }
    }
}

/// 续命节奏。必须明显小于巡查周期的下限（`AiService::start` 里那个 `max(1s)`），
/// 否则续得比查得慢，等于没续。
const KEEP_ALIVE_TICK: Duration = Duration::from_millis(500);

async fn janitor_loop(service: std::sync::Weak<AiService>, tick: Duration) {
    loop {
        tokio::time::sleep(tick).await;
        let Some(service) = service.upgrade() else {
            return;
        };
        reap_if_idle(&service, SlotKind::Chat).await;
        reap_if_idle(&service, SlotKind::Embedding).await;
    }
}

async fn reap_if_idle(service: &AiService, kind: SlotKind) {
    let mut slot = service.slot_mutex(kind).lock().await;
    let idle = slot
        .running
        .as_ref()
        .map(|r| {
            r.last_used
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed()
                >= service.idle_timeout
        })
        .unwrap_or(false);
    if idle {
        if let Some(running) = slot.running.take() {
            running.proc.shutdown().await;
            service.emit(kind, AiEngineStatus::Stopped, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{require_llama_server, require_tiny_model};
    use aa4c_engine::ProcessSpawner;

    /// 请求在飞的整个期间，`last_used` 必须一直被刷新——否则巡查任务会在请求
    /// 走完之前把进程杀掉（见 [`keep_alive_while`]）。
    ///
    /// 不起真进程：这里要验的是续命机制本身，用一个"慢 future"代替慢推理即可。
    /// 判据是**请求进行当中** `last_used` 最久落后了多少：续着命的话不会超过一个
    /// 续命周期多一点；不续的话会一路涨到整个请求时长。
    /// **验证过它抓得住**：把 `keep_alive_while` 里 `ticker.tick()` 那个分支删掉，
    /// 落后量立刻涨到 ≈2s，断言变红。
    #[tokio::test]
    async fn a_request_in_flight_keeps_its_slot_alive() {
        let last_used = Arc::new(StdMutex::new(Instant::now()));
        let watcher = last_used.clone();
        let worst = Arc::new(StdMutex::new(Duration::ZERO));
        let recorder = worst.clone();
        let probe = tokio::spawn(async move {
            for _ in 0..9 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let behind = watcher.lock().unwrap().elapsed();
                let mut w = recorder.lock().unwrap();
                if behind > *w {
                    *w = behind;
                }
            }
        });

        keep_alive_while(&last_used, tokio::time::sleep(Duration::from_secs(2))).await;
        probe.await.unwrap();

        let behind = *worst.lock().unwrap();
        assert!(
            behind < Duration::from_secs(1),
            "请求进行中 last_used 最久落后了 {behind:?}——续命没生效，\
             巡查任务会在这段时间里把进程当成空闲杀掉"
        );
    }

    /// 未配置模型的槽位直接 `Unavailable`，不尝试拉起进程——同下载能力
    /// 缺失时的既有降级语义。
    #[tokio::test]
    async fn slot_without_configured_model_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let spawner: Arc<dyn SidecarSpawner> = Arc::new(ProcessSpawner::new("does-not-matter"));
        let (events, _rx) = broadcast::channel(16);
        let service = AiService::start(
            spawner,
            AiConfig {
                chat_model: None,
                embedding_model: None,
                idle_timeout: Duration::from_secs(600),
                state_dir: dir.path().to_path_buf(),
            },
            events,
        );
        let err = service
            .chat_completion(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, Aa4cError::Unavailable(_)));
    }

    /// 真实进程全链路：懒启动（第一次请求前进程不存在）→ 首次请求拉起 →
    /// 健康 → 短空闲超时后巡查任务自动杀掉释放内存 → PID 文件清理干净。
    /// 不 mock，真实 `llama-server` + 真实微型模型。
    #[tokio::test]
    async fn lazy_starts_on_first_request_and_idle_reaper_stops_it() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let dir = tempfile::tempdir().unwrap();
        let spawner: Arc<dyn SidecarSpawner> = Arc::new(ProcessSpawner::new(bin));
        let (events, mut rx) = broadcast::channel(16);

        let service = AiService::start(
            spawner,
            AiConfig {
                chat_model: Some(model),
                embedding_model: None,
                // 历史：500ms→2s 在本机都稳，真实 CI 上连续两次炸在这个精确位置
                // ——`aa4c-ai` 一个测试二进制里好几个用例并发拉起真实 llama-server
                // 抢共享 runner 的 CPU，一次推理的端到端延迟能拖过给的余量，巡查
                // 任务就在请求还没走完时把进程杀了。当时是靠一路加时间绕过去的
                // （最后落在 10s）。
                //
                // **那个竞态本身已经修掉了**（`keep_alive_while`：请求在飞的整个
                // 期间持续续命），所以这里不必再为它留余量。10s 保留下来只是给
                // 这条用例自己的"拉起 + 一次推理"留出共享 runner 上的执行时间，
                // **再遇到类似失败不要靠继续加这个数字解决**——先确认是不是又有
                // 哪条路径漏了续命。
                idle_timeout: Duration::from_secs(10),
                state_dir: dir.path().to_path_buf(),
            },
            events,
        );

        let resp = service
            .chat_completion(serde_json::json!({
                "messages": [{"role": "user", "content": "hi"}],
                "stream": false,
                "max_tokens": 4
            }))
            .await
            .unwrap();
        assert!(resp["choices"][0]["message"]["content"].is_string());

        let mut saw_starting = false;
        let mut saw_ready = false;
        while let Ok(Ok(CoreEvent::AiEngineState {
            slot: AiSlot::Chat,
            status,
            ..
        })) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
        {
            if status == AiEngineStatus::Starting {
                saw_starting = true;
            }
            if status == AiEngineStatus::Ready {
                saw_ready = true;
            }
        }
        assert!(saw_starting && saw_ready);

        // 空闲超时是 10s，巡查 tick 同样是 10s（`idle_timeout.min(30s).max(1s)`）——
        // 最坏情况下要等 idle_timeout + 一个 tick 周期才会被回收，给足 25 秒。
        tokio::time::sleep(Duration::from_secs(25)).await;
        assert!(
            !dir.path().join("llama-chat.pid").exists(),
            "idle reaper should have stopped the slot"
        );

        let mut saw_stopped = false;
        while let Ok(Ok(CoreEvent::AiEngineState {
            slot: AiSlot::Chat,
            status: AiEngineStatus::Stopped,
            ..
        })) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
        {
            saw_stopped = true;
        }
        assert!(
            saw_stopped,
            "should have observed a Stopped event after idle reap"
        );
    }
}
