//! AA4C Core：应用生命周期、事件总线、服务编排（API_DESIGN.md §8）。
//!
//! Core 只协调，不实现业务（AGENTS.md Core 规则）：
//! 装配 identity / store / discovery / transfer / pairing 五个组件，
//! 用一条 broadcast 事件总线把它们串起来，并对 Tauri 层暴露统一的编排方法。

#![forbid(unsafe_code)]

mod dispatch;
mod introduce;
mod local_server;
mod orchestrate;

/// 插件边界（住在独立的 `aa4c-plugin` crate：插件要实现它，
/// 而核心不能反过来依赖插件——trait 放核心里就成了循环）。
pub use aa4c_plugin as plugin;
mod portmap;
mod reach;
mod server_link;
mod settings;
mod sync_exchange;
mod sync_index;
mod unified;

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_discovery::DiscoveryService;
use aa4c_identity::{Identity, PairingManager};
use aa4c_plugin::{PluginContextBuilder, PluginRegistry};
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
    /// 本次构建装配了哪些插件（ARCHITECTURE.md 原则 3）。
    ///
    /// 此前这里是三个具体的 `SidecarSpawner`（aria2 / Transmission / llama-server）——
    /// 核心的配置结构体直接点名了两个下载引擎和一个推理引擎。现在壳层自己构造
    /// `DownloadPlugin` / `ArchivePlugin`（只有它拿得到基于 Tauri 的 spawner），
    /// 注册进来即可；核心不认识其中任何一个。
    ///
    /// 空注册表 = 一个只有连接能力的构建，完全合法。
    pub plugins: PluginRegistry,
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
            plugins: PluginRegistry::default(),
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
    /// 本次构建装配的插件（下载 / 归档与 AI / …）。
    ///
    /// 此前是 `download` / `ai` / `suggest` / `kb` 四个具体字段。
    pub plugins: PluginRegistry,
    /// 每台设备当下的可达性（V0.8 F2）。只存内存——可达性是**当下**的属性，
    /// 重启后拿昨天的结论糊弄用户比说「不知道」更糟（见 `reach` 模块文档）。
    reach: reach::ReachState_,
    events: EventSender,
    self_info: DeviceInfo,
    listen_port: u16,
    /// 平台注入的缺省接收目录（用户未设置时 get_settings 的回落值）。
    save_dir_fallback: String,
    /// 唤醒自建服务器常驻连接立即重新注册（里程碑 C3，见 `server_link::spawn_register_loop`）。
    register_notify: Arc<tokio::sync::Notify>,
    /// 内置服务器（里程碑 R4）：`None` 表示当前没开（设置里关着，或者端口被占没起来）。
    local_server: local_server::LocalServer,
    /// 内置服务器端口的 UPnP 映射结果（里程碑 R4）：拿它给用户显示一个真能用的对外地址。
    server_portmap: portmap::PortMapState,
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
        let current = settings::load(
            &store,
            &fallback_name,
            &save_dir_fallback,
            &config.plugins.ids(),
        )
        .await?;

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
        //    不再局限于 mDNS 在线快照，远程设备靠周期定时器兜底，见 sync_exchange 模块文档）。
        //    V0.8 F2 起这条轮次顺带记录可达性——它本来就在对每台设备走完整连接阶梯，
        //    不必另开一条 ping 循环（见 `reach` 模块文档）。
        let reach = reach::ReachState_::default();
        sync_exchange::spawn_exchange_loop(
            sync_exchange::ExchangeCtx {
                store: store.clone(),
                discovery: discovery.clone(),
                identity: identity.clone(),
                fallback_name: fallback_name.clone(),
                fallback_save_dir: save_dir_fallback.clone(),
                transfer: transfer.clone(),
                events: events.clone(),
                reach: reach.clone(),
            },
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
        // 9d. 内置可选 server 模式（TRUST_DESIGN.md §6.3，里程碑 R4）：让家里那台常开的
        //     设备兼任汇合点，门槛从「要有 VPS」降到「家里有台常开设备」。默认关闭；
        //     打开后按设置启停，停机时把端口还回去。**诚实前提**：用户仍然需要一个稳定
        //     入口（DDNS 或固定地址）才能被找到，见 local_server 模块文档。
        let local_server = local_server::LocalServer::new(config.data_dir.clone());
        local_server::spawn_local_server_loop(
            store.clone(),
            fallback_name.clone(),
            save_dir_fallback.clone(),
            local_server.clone(),
            shutdown.clone(),
        );

        let portmap_state = portmap::PortMapState::default();
        let server_portmap_state = portmap::PortMapState::default();
        let upnp = if config.disable_port_mapping {
            None
        } else {
            portmap::UpnpMapper::new(server_link::primary_local_ip_v4())
        };
        match upnp {
            Some(mapper) => {
                let mapper: Arc<dyn portmap::PortMapper> = Arc::new(mapper);
                // 传输端口：闸是 `enable_remote`
                {
                    let store = store.clone();
                    let name = fallback_name.clone();
                    let dir = save_dir_fallback.clone();
                    portmap::spawn_portmap_loop(
                        move || {
                            let (store, name, dir) = (store.clone(), name.clone(), dir.clone());
                            async move {
                                portmap::transfer_target(&store, &name, &dir, actual_port).await
                            }
                        },
                        mapper.clone(),
                        portmap_state.clone(),
                        shutdown.clone(),
                    );
                }
                // 内置服务器端口：闸是 `enable_local_server`，端口还是动态的（里程碑 R4）
                {
                    let store = store.clone();
                    let name = fallback_name.clone();
                    let dir = save_dir_fallback.clone();
                    let local = local_server.clone();
                    portmap::spawn_portmap_loop(
                        move || {
                            let (store, name, dir, local) =
                                (store.clone(), name.clone(), dir.clone(), local.clone());
                            async move {
                                portmap::local_server_target(&store, &name, &dir, &local).await
                            }
                        },
                        mapper,
                        server_portmap_state.clone(),
                        shutdown.clone(),
                    );
                }
            }
            None => tracing::debug!("no outbound local ip, port mapping unavailable"),
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

        // 11. 插件（ARCHITECTURE.md 原则 3）：下载中心、归档与 AI 都在这一步拉起。
        //     此前这里是三段并排的具体装配——aria2/Transmission 的 `DownloadService`、
        //     llama-server 的 `AiService`，外加借用它的 `SuggestEngine`/`KbService`——
        //     加起来近 90 行，`Core` 因此认识两个下载引擎和一个推理引擎。现在壳层
        //     自己构造插件，核心只负责按顺序拉起。
        //
        //     单个插件起不来只记 warn、不返回 Err：一个下载引擎连不上不该让设备连不上
        //     （沿用此前「可选能力整体降级不阻塞 Core 启动」的一贯设计）。
        let plugin_ctx = PluginContextBuilder {
            store: store.clone(),
            events: events.clone(),
            data_dir: config.data_dir.clone(),
            settings: current.plugins.clone(),
        };
        config.plugins.start_all(&plugin_ctx).await;
        let plugins = config.plugins;

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
            plugins,
            reach,
            events,
            self_info,
            listen_port: actual_port,
            save_dir_fallback,
            register_notify,
            local_server,
            server_portmap: server_portmap_state,
            shutdown,
        }))
    }

    /// 优雅关闭：停止 mDNS 广播与浏览（注销服务、清空在线表）+ 下载引擎优雅退出。
    pub async fn shutdown(&self) -> Result<()> {
        // 先断信号：后台循环各自在下一个 select 点退出，accept 循环随即释放监听端口。
        self.shutdown.cancel();
        self.transfer.shutdown();
        self.discovery.stop().await?;
        self.plugins.shutdown_all().await;
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
