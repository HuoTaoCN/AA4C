//! AA4C Core：应用生命周期、事件总线、服务编排（API_DESIGN.md §8）。
//!
//! Core 只协调，不实现业务（AGENTS.md Core 规则）：
//! 装配 identity / store / discovery / transfer / pairing 五个组件，
//! 用一条 broadcast 事件总线把它们串起来，并对 Tauri 层暴露统一的编排方法。

#![forbid(unsafe_code)]

mod dispatch;
mod orchestrate;
mod settings;

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_discovery::DiscoveryService;
use aa4c_identity::{Identity, PairingManager};
use aa4c_store::Store;
use aa4c_transfer::{TransferConfig, TransferService};
use aa4c_types::{CoreEvent, DeviceInfo, Platform, Result, DEFAULT_PORT};
use tokio::sync::broadcast;

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
    events: EventSender,
    self_info: DeviceInfo,
    listen_port: u16,
    /// 平台注入的缺省接收目录（用户未设置时 get_settings 的回落值）。
    save_dir_fallback: String,
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
        discovery.start(actual_port).await?;

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
            events,
            self_info,
            listen_port: actual_port,
            save_dir_fallback,
        }))
    }

    /// 优雅关闭：停止 mDNS 广播与浏览（注销服务、清空在线表）。
    pub async fn shutdown(&self) -> Result<()> {
        self.discovery.stop().await?;
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
