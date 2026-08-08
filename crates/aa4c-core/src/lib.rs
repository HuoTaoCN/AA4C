//! AA4C Core：应用生命周期、事件总线、服务编排（API_DESIGN.md §8）。
//!
//! Core 只协调，不实现业务（AGENTS.md Core 规则）：
//! 装配 identity / store / discovery / transfer / pairing 五个组件，
//! 用一条 broadcast 事件总线把它们串起来，并对 Tauri 层暴露统一的编排方法。

#![forbid(unsafe_code)]

mod archive;
mod dispatch;
mod introduce;
mod orchestrate;
mod portmap;
mod server_link;
mod settings;
mod sync_exchange;
mod sync_index;
mod unified;

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_ai::{AiService, KbService, SuggestEngine};
use aa4c_discovery::DiscoveryService;
use aa4c_download::{DownloadService, SidecarSpawner};
use aa4c_identity::{Identity, PairingManager};
use aa4c_store::Store;
use aa4c_transfer::{TransferConfig, TransferService};
use aa4c_types::{CoreEvent, DeviceInfo, Platform, Result, DEFAULT_PORT};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// AA4C 版本号（与 workspace 版本一致）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 事件总线发送端。
pub type EventSender = broadcast::Sender<CoreEvent>;

/// 事件总线缓冲容量（慢订阅者落后会丢最旧事件，UI 启动后重新拉取快照）。
const EVENT_CAPACITY: usize = 256;

/// Core 启动配置。
pub struct CoreConfig {
    /// 数据目录（身份、数据库的根；桌面取 dirs，Android 由 Tauri 注入）。
    pub data_dir: PathBuf,
    /// 本机设备名；`None` 时取已保存设置或 hostname。
    pub device_name: Option<String>,
    /// 期望监听端口（占用时由传输层自动递增）。
    pub listen_port: u16,
    /// 传输引擎配置（接收目录会被设置项覆盖）。
    pub transfer: TransferConfig,
    /// 下载引擎（aria2，HTTP/HTTPS/FTP，里程碑 D1）子进程拉起器
    /// （DOWNLOAD_DESIGN.md §2）：桌面壳层注入基于 `tauri-plugin-shell` 的实现，
    /// `None` 时下载能力整体不存在（Android 等未接入的平台/构建，V0.4 范围）——
    /// 与"注入了但 aria2c 启动失败"的降级（仍是 `Some`，只是内部 `cmd_tx` 为空）
    /// 是两种不同的不可用，后者由 `DownloadService::start` 自己处理。这个字段
    /// 是"本平台是否支持下载能力"的总闸——`bt_spawner` 只决定 BT 这一个引擎
    /// 自己的可用性，不单独决定整个下载中心存不存在。
    pub download_spawner: Option<Arc<dyn SidecarSpawner>>,
    /// BT 引擎（Transmission，Magnet，里程碑 D2）子进程拉起器
    /// （DOWNLOAD_DESIGN.md §3.6）：与 `download_spawner` 是两个独立的可选注入，
    /// 各自的启动/健康检查失败互不影响对方——`None` 时只是 BT 能力不可用，
    /// HTTP/HTTPS/FTP 直链正常工作。
    pub bt_spawner: Option<Arc<dyn SidecarSpawner>>,
    /// AI 引擎（llama-server，里程碑 AI2）子进程拉起器（ARCHIVE_DESIGN.md §3.2）：
    /// 与下载引擎完全独立的另一个可选注入——`None` 时 AI 能力整体不存在（同
    /// `download_spawner` 的总闸语义）；`Some` 之后具体槽位是否真的可用还要看
    /// `ai_chat_model`/`ai_embedding_model` 有没有配置模型文件（`AiService`
    /// 内部处理，同下载能力"注入了但没配置/起不来"的既有降级设计）。
    pub ai_spawner: Option<Arc<dyn SidecarSpawner>>,
    /// 不启动 mDNS 广播 / 浏览（**测试专用开关**，同 `TransferConfig::disable_punch`
    /// 的既有惯例）。
    ///
    /// `DiscoveryService` 仍然会建出来（`list_devices` 等照常查它，只是永远为空），
    /// 只是不 `start()`。集成测试全部用显式地址建连，mDNS 对它们只是噪声：每个
    /// `ServiceDaemon` 自带一条 OS 线程和一组 5353 组播 socket，并行跑十几个用例时
    /// 几十个守护进程互相挤，配对会开始超时（此前套件整套跑不通的主因之一）。
    pub disable_discovery: bool,
    /// 不做 UPnP 自动端口映射（**测试专用开关**，同 `disable_discovery` 的既有惯例）。
    ///
    /// 端口映射的真实实现**会去改开发者自己路由器的配置**。而集成测试里有若干用例会打开
    /// `enable_remote`（`portmap` 的外层闸），一旦它们的运行时间越过退避周期，测试就会在
    /// 跑测试的人家里真开一个端口——这是不可接受的，也不能靠"反正跑得快"来指望。
    /// 集成测试一律把这一项设为 `true`，从结构上堵死，而不是依赖时序上的巧合。
    pub disable_port_mapping: bool,
}

impl CoreConfig {
    /// 用平台默认值构造（监听端口 42420、设备名取 hostname）。
    ///
    /// 接收目录缺省为桌面下载目录下的 `AA4C`；Android 等平台应在创建后用
    /// Tauri path resolver 覆盖 `transfer.default_save_dir`（API_DESIGN §11）。
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            device_name: None,
            listen_port: DEFAULT_PORT,
            transfer: TransferConfig {
                default_save_dir: settings::default_save_dir(),
                ..TransferConfig::default()
            },
            download_spawner: None,
            bt_spawner: None,
            ai_spawner: None,
            disable_discovery: false,
            disable_port_mapping: false,
        }
    }
}

/// 应用核心：持有全部服务句柄与事件总线。
pub struct Core {
    pub identity: Arc<Identity>,
    pub store: Store,
    pub discovery: Arc<DiscoveryService>,
    pub transfer: Arc<TransferService>,
    pub pairing: Arc<PairingManager>,
    /// 下载中心服务（里程碑 D1）；`None` = 本平台/构建未接入下载能力
    /// （与"接入了但 aria2c 起不来"的降级是两种不同的不可用，见 `CoreConfig` 文档）。
    pub download: Option<Arc<DownloadService>>,
    /// AI 引擎服务（里程碑 AI2）；`None` = 本平台/构建未接入 AI 能力（与
    /// `download` 的既有语义一致，见 `CoreConfig::ai_spawner` 文档）。
    pub ai: Option<Arc<AiService>>,
    /// AI 标签/分类建议批量队列（里程碑 AI3，ARCHIVE_DESIGN.md §5）；`None` 同
    /// `ai` 的既有语义——没有 AI 引擎就没有建议能力。
    pub suggest: Option<Arc<SuggestEngine>>,
    /// 本地知识库（里程碑 AI4，ARCHIVE_DESIGN.md §6）；`None` 同 `ai` 的既有语义
    /// ——没有 AI 引擎就没有嵌入能力，知识库也就无从谈起。
    pub kb: Option<Arc<KbService>>,
    events: EventSender,
    self_info: DeviceInfo,
    listen_port: u16,
    /// 平台注入的缺省接收目录（用户未设置时 get_settings 的回落值）。
    save_dir_fallback: String,
    /// 唤醒自建服务器常驻连接立即重新注册（里程碑 C3，见 `server_link::spawn_register_loop`）。
    register_notify: Arc<tokio::sync::Notify>,
    /// 停机信号：[`Self::shutdown`] 触发后本机的全部常驻后台循环退出。
    ///
    /// 此前 `shutdown()` 只停了 discovery / download / AI，而同步扫描、索引交换、引荐、
    /// 自建服务器续约这四条循环，以及传输层的 accept 循环都没有出口——一个 `Core` 停掉
    /// 之后它们照跑不误，端口也不释放。桌面端因为进程随后就退出而看不出来，但同进程内
    /// 反复起停 `Core` 的场景（集成测试整套跑、将来的应用内重启）会一路堆积。
    shutdown: CancellationToken,
}

impl Core {
    /// 完整启动序列：身份 → 数据库 → 遗留任务清理 → 配对/传输装配 →
    /// 传输监听 → mDNS 广播。返回可共享的 `Arc<Core>`。
    pub async fn start(config: CoreConfig) -> Result<Arc<Core>> {
        // 1. 身份（首次自动生成 Ed25519 密钥）
        let identity = Arc::new(Identity::load_or_generate(&config.data_dir)?);

        // 2. 数据库（自动迁移）
        let store = Store::open(&config.data_dir.join("aa4c.db")).await?;

        // 3. 启动清理：上次运行遗留的未完成任务标记失败
        match store.fail_incomplete_tasks().await {
            Ok(n) if n > 0 => tracing::info!(count = n, "marked stale tasks as failed"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "stale task cleanup failed"),
        }

        // 设置：缺省补齐。设备名优先级 已保存设置 > config > hostname
        let fallback_name = config
            .device_name
            .clone()
            .unwrap_or_else(settings::default_device_name);
        // 平台注入的缺省接收目录（桌面=下载目录，Android=应用可写目录）
        let save_dir_fallback = config
            .transfer
            .default_save_dir
            .to_string_lossy()
            .into_owned();
        let current = settings::load(&store, &fallback_name, &save_dir_fallback).await?;

        let self_info = DeviceInfo {
            id: identity.device_id().clone(),
            name: current.device_name.clone(),
            platform: Platform::current(),
            version: VERSION.to_string(),
            addr: None,
            online: true,
            trusted: true,
            trust_level: None,
        };

        // 事件总线：发送端常驻，订阅端由 UI / 内部任务按需创建
        let (events, _initial_rx) = broadcast::channel(EVENT_CAPACITY);

        // 4. 配对管理器
        let pairing = Arc::new(PairingManager::new(
            identity.clone(),
            self_info.clone(),
            store.clone(),
            events.clone(),
        ));

        // 5. 传输服务（接收目录取设置项），注入配对分流钩子
        let mut transfer_config = config.transfer;
        transfer_config.default_save_dir = PathBuf::from(&current.save_dir);
        let transfer = TransferService::new(
            identity.clone(),
            store.clone(),
            events.clone(),
            transfer_config,
        );
        transfer.set_pair_dispatch(Arc::new(dispatch::PairDispatch::new(pairing.clone())));
        transfer.set_index_dispatch(Arc::new(dispatch::IndexServe::new(store.clone())));
        transfer.set_fetch_resolver(Arc::new(dispatch::FetchServe::new(store.clone())));
        transfer.set_share_resolver(Arc::new(dispatch::ShareServe::new(store.clone())));
        // 中继拨号器（连接阶梯第 4 档，里程碑 C3）：未开启远程/未配置服务器时其内部
        // 会自行报错，不影响任何现有行为（同其余注入钩子的「未注入即不启用」惯例）。
        transfer.set_relay_dialer(Arc::new(server_link::RelayDialerImpl::new(
            store.clone(),
            identity.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
        )));

        // 6. 启动监听（端口占用自动递增，返回真实端口）
        //    端口优先级：显式覆盖（config 非默认值，如测试用 0）> 已保存设置
        let port = if config.listen_port == DEFAULT_PORT {
            current.listen_port
        } else {
            config.listen_port
        };
        let actual_port = transfer.start_listener(port).await?;

        // 7. mDNS 广播 + 浏览（用真实端口）
        let discovery = Arc::new(DiscoveryService::new(self_info.clone(), events.clone())?);
        if !config.disable_discovery {
            discovery.start(actual_port).await?;
        }

        // 8. 同步：Inbox 落点 + 初始扫描，并启动后台扫描循环（SYNC_DESIGN.md §3/§5，里程碑 2）
        match store.ensure_inbox_scope(&current.save_dir).await {
            Ok(inbox) => {
                if let Err(e) = sync_index::scan_scope(&store, &inbox).await {
                    tracing::warn!(error = %e, "initial sync scan failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "ensure inbox scope failed"),
        }
        // 一个停机信号贯穿下面全部常驻循环，`shutdown()` 一次全停（见 `Core::shutdown`）。
        let shutdown = CancellationToken::new();
        sync_index::spawn_background_scan(store.clone(), events.clone(), shutdown.clone());

        // 9. 跨设备索引交换：与全部完全信任设备交换索引摘要（里程碑 3；里程碑 C4 起
        //    不再局限于 mDNS 在线快照，远程设备靠周期定时器兜底，见 sync_exchange 模块文档）
        sync_exchange::spawn_exchange_loop(
            store.clone(),
            discovery.clone(),
            identity.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
            transfer.clone(),
            events.clone(),
            shutdown.clone(),
        );

        // 9b. 信任传递 / 引荐（TRUST_DESIGN.md §5，里程碑 R2）：与全部完全信任设备交换
        //     「这些也是我的设备」的指纹。刻意与索引交换分开跑——引荐几乎不变，用不着
        //     30s 一轮（见 introduce 模块文档）。只落待确认，绝不自动信任。
        introduce::spawn_introduce_loop(
            store.clone(),
            discovery.clone(),
            identity.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
            transfer.clone(),
            events.clone(),
            shutdown.clone(),
        );

        // 9c. 自动端口映射（UPnP IGD，TRUST_DESIGN.md §6.2，里程碑 R3）：在路由器上给
        //     本机传输端口开 TCP+UDP 两条映射，让有公网 IPv4 的家宽也能被直连命中连接
        //     阶梯第 2 档。两层闸都开才动手（`enable_remote` 默认关 + `enable_port_mapping`
        //     默认开），停机时拆掉——不能在用户的路由器上留洞。见 portmap 模块文档。
        //
        //     映射到的公网地址经 `PortMapState` 交给下面的注册/打洞两条路径当候选上报，
        //     不接这一步的话映射了也没人知道，等于白做。
        let portmap_state = portmap::PortMapState::default();
        let upnp = if config.disable_port_mapping {
            None
        } else {
            portmap::UpnpMapper::new(server_link::primary_local_ip_v4())
        };
        if let Some(mapper) = upnp {
            portmap::spawn_portmap_loop(
                store.clone(),
                fallback_name.clone(),
                save_dir_fallback.clone(),
                actual_port,
                Arc::new(mapper),
                portmap_state.clone(),
                shutdown.clone(),
            );
        } else {
            tracing::debug!("no outbound local ip, port mapping unavailable");
        }

        // 10. 自建服务器注册续约（CONNECT_DESIGN.md §3.2，里程碑 C2）：未开启远程 /
        //     未配置服务器时循环内部直接跳过，不影响任何现有行为
        let (register_notify, signal_channel) = server_link::spawn_register_loop(
            store.clone(),
            identity.clone(),
            actual_port,
            fallback_name.clone(),
            save_dir_fallback.clone(),
            transfer.clone(),
            portmap_state.clone(),
            shutdown.clone(),
        );
        // 打洞拨号器（连接阶梯第 3 档，里程碑 C5）：同中继拨号器，未开启远程/未配置
        // 服务器时其内部会自行报错，不影响任何现有行为。
        transfer.set_punch_dialer(Arc::new(server_link::PunchDialerImpl::new(
            store.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
            actual_port,
            transfer.clone(),
            signal_channel,
            portmap_state,
        )));

        // 11. 下载中心（DOWNLOAD_DESIGN.md，里程碑 D1 aria2 + D2 Transmission）：只在
        //     壳层注入了 aria2 spawner 时才尝试拉起下载中心（这是"本平台支不支持
        //     下载能力"的总闸，Android 等未接入的平台/构建保持 `None`）；
        //     `bt_spawner` 独立传入，两个引擎各自的启动/健康检查失败互不影响
        //     对方，也都不返回 Err——下载能力（或其中一个引擎）整体降级不可用，
        //     不阻塞 Core 启动（`DownloadService::start` 内部处理，同其余可选
        //     能力的一贯降级设计）。
        let download = match config.download_spawner {
            Some(spawner) => Some(
                DownloadService::start(
                    spawner,
                    config.bt_spawner,
                    store.clone(),
                    events.clone(),
                    config.data_dir.clone(),
                    PathBuf::from(&current.download_dir),
                    aa4c_download::DownloadLimits {
                        speed_limit_kbps: current.download_speed_limit_kbps,
                        upload_limit_kbps: current.download_upload_limit_kbps,
                        concurrency: current.download_concurrency,
                        max_connections_per_file: current.download_max_connections_per_file,
                        user_agent: current.download_user_agent.clone(),
                        proxy: current.download_proxy.clone(),
                        proxy_bypass: current.download_proxy_bypass.clone(),
                        bt_ratio_limit: current.bt_ratio_limit,
                        bt_idle_seeding_limit_minutes: current.bt_idle_seeding_limit_minutes,
                        bt_trackers: current.bt_trackers.clone(),
                    },
                )
                .await,
            ),
            None => None,
        };

        // 11b. 启动时自动继续未完成的下载（对标 Motrix 的 `resume-all-when-app-launched`，
        //      默认关闭）。放在 `DownloadService::start` 之后——那里面已经跑完了首轮
        //      对账（`reconcile`），此刻库里的状态才是引擎的真实状态，直接 `resume_all`
        //      不会去恢复一个引擎根本不认识的任务。后台 spawn 而不是 `.await`：逐个
        //      任务发 RPC 在任务多时不该拖慢 Core 启动（同其余可选能力"不阻塞启动"
        //      的一贯设计）。
        if current.download_resume_on_start {
            if let Some(svc) = download.clone() {
                tokio::spawn(async move {
                    let n = svc.resume_all().await;
                    if n > 0 {
                        tracing::info!(count = n, "resumed unfinished downloads on startup");
                    }
                });
            }
        }

        // 12. 归档（ARCHIVE_DESIGN.md，里程碑 AI1）：首次启动写入五条停用的预设规则，
        //     再起下载完成钩子（DownloadDone → 跑规则引擎，见 archive 模块文档）。
        if let Err(e) = archive::engine::ensure_default_rules(&store).await {
            tracing::warn!(error = %e, "ensure default archive rules failed");
        }
        archive::spawn_download_hook(
            store.clone(),
            events.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
        );

        // 13. AI 引擎（ARCHIVE_DESIGN.md §3，里程碑 AI2）：懒启动，`AiService::start`
        //     本身不拉起任何进程，只登记配置——同下载中心一样，注入了 spawner 但没
        //     配置模型/进程起不来都不阻塞 Core 启动（`AiService` 内部按需处理，见
        //     `ensure_running` 的 `Unavailable` 语义）。PID 文件放数据目录下的
        //     `ai-state/`（不与归档/同步等其他子目录混放）。
        let ai = config.ai_spawner.map(|spawner| {
            aa4c_ai::AiService::start(
                spawner,
                aa4c_ai::AiConfig {
                    chat_model: current.ai_chat_model.clone().map(PathBuf::from),
                    embedding_model: current.ai_embedding_model.clone().map(PathBuf::from),
                    idle_timeout: std::time::Duration::from_secs(
                        u64::from(current.ai_idle_timeout_minutes) * 60,
                    ),
                    state_dir: config.data_dir.join("ai-state"),
                },
                events.clone(),
            )
        });
        // AI 标签/分类建议（里程碑 AI3，ARCHIVE_DESIGN.md §5）：门闩条件跟 `ai` 一致
        // （没有 AI 引擎就不可能出建议），`SuggestEngine::new` 借用 `ai` 的对话槽位，
        // 不单独起进程/占资源。
        let suggest = ai
            .clone()
            .map(|ai| aa4c_ai::SuggestEngine::new(ai, events.clone()));
        // 本地知识库（里程碑 AI4，ARCHIVE_DESIGN.md §6）：同 `suggest` 一样门闩
        // 条件跟 `ai` 一致，借用同一个 `AiService`（对话槽位问答、嵌入槽位摄入/
        // 检索），不单独起进程。`store.clone()` 廉价（内部只是一个 mpsc Sender）。
        let kb = ai
            .clone()
            .map(|ai| aa4c_ai::KbService::new(ai, store.clone(), events.clone()));

        tracing::info!(
            device = %self_info.name,
            id = %self_info.id,
            port = actual_port,
            "AA4C core started"
        );
        Ok(Arc::new(Core {
            identity,
            store,
            discovery,
            transfer,
            pairing,
            download,
            ai,
            suggest,
            kb,
            events,
            self_info,
            listen_port: actual_port,
            save_dir_fallback,
            register_notify,
            shutdown,
        }))
    }

    /// 优雅关闭：停止 mDNS 广播与浏览（注销服务、清空在线表）+ 下载引擎优雅退出。
    pub async fn shutdown(&self) -> Result<()> {
        // 先断信号：后台循环各自在下一个 select 点退出，accept 循环随即释放监听端口。
        self.shutdown.cancel();
        self.transfer.shutdown();
        self.discovery.stop().await?;
        if let Some(download) = &self.download {
            download.shutdown().await;
        }
        if let Some(ai) = &self.ai {
            ai.shutdown().await;
        }
        tracing::info!("AA4C core shut down");
        Ok(())
    }

    /// 订阅事件总线（UI / 内部任务用）。
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    /// 本机设备信息快照。
    pub fn self_info(&self) -> DeviceInfo {
        self.self_info.clone()
    }

    /// 传输服务实际监听端口（端口递增后的真实值，mDNS 广播用此端口）。
    pub fn listen_port(&self) -> u16 {
        self.listen_port
    }
}
