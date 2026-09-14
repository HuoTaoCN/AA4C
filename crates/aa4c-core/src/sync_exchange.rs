//! 跨设备索引交换（SYNC_DESIGN.md §3.3，里程碑 3；里程碑 C4 接入完整连接阶梯）。
//!
//! 与**完全信任**设备交换索引摘要（只取元数据），落 `remote_index`，据此在统一视图里
//! 把对端文件标黄（可下载）/标红（离线）。按需拉取内容是里程碑 4。
//!
//! 触发策略：
//! - 启动时、以及此后每逢 `DeviceFound`（mDNS 上线/重连）：对当前全部完全信任设备各拉一次；
//! - 另加一条**周期定时器**（`REMOTE_REFRESH_INTERVAL`）：`DeviceFound` 只对 mDNS 能发现的
//!   局域网设备触发，远程（跨网络、只能靠自建服务器 + 中继连到的）完全信任设备永远不会
//!   产生这个事件，没有周期兜底就永远不会刷新——这正是里程碑 C4 要补的缺口（此前
//!   `refresh_online` 只遍历 `discovery.devices()`，跳过所有非局域网设备）。
//!
//! 对端解析统一走 [`crate::orchestrate::resolve_addr`]（mDNS → 落库最后地址 → 服务器
//! Lookup），解析不出地址也照样尝试——`TransferService::fetch_index` 会在其上再落到
//! 中继兜底（见 `TransferService::dial`），是否真的可达最终看这次调用是否成功。

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa4c_discovery::DiscoveryService;
use aa4c_identity::Identity;
use aa4c_store::Store;
use aa4c_transfer::TransferService;
use aa4c_types::{CoreEvent, DeviceId, RemoteIndexEntry, Result, TrustLevel};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::orchestrate::resolve_addr;
use crate::reach::{self, ReachState_};
use crate::EventSender;

/// 索引交换要用到的一整套依赖。
///
/// 这七样东西在本模块里**每个函数都要、而且总是一起出现**，此前是逐个传参——
/// `fetch_one` 与 `refresh_all_full_trust` 都因此挂着
/// `#[allow(clippy::too_many_arguments)]`。F2 要再加一个「可达性状态」，
/// 与其把参数加到九个，不如把它们收成一个上下文（AGENTS.md：简单 > 复杂）。
pub(crate) struct ExchangeCtx {
    pub store: Store,
    pub discovery: Arc<DiscoveryService>,
    pub identity: Arc<Identity>,
    pub fallback_name: String,
    pub fallback_save_dir: String,
    pub transfer: Arc<TransferService>,
    pub events: EventSender,
    /// V0.8 F2：探测结果记在这里，不新开循环——理由见 `crate::reach` 模块文档。
    pub reach: ReachState_,
}

/// 周期性全量刷新的间隔：`DeviceFound` 覆盖不到远程设备，靠这个兜底发现/重连
/// （个人自托管场景，几台设备的轮询开销可忽略；不与 `aa4c_server::REGISTER_TTL` 绑定，
/// 两者关注点不同——那是"我还在不在线"，这是"我要不要去问问对方"）。
const REMOTE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// 与单台设备交换索引：仅对完全信任设备生效，成功后广播 `SyncIndexUpdated`。
/// 返回 `true` 表示确实拉取并更新了远端索引。
///
/// **顺带记录可达性**（V0.8 F2）：这条调用走的是完整连接阶梯，成功与否、走的哪一档，
/// 正是首页设备状态图要显示的东西。不为它另开探测循环，理由见 `crate::reach`。
pub(crate) async fn fetch_one(ctx: &ExchangeCtx, device_id: &DeviceId) -> Result<bool> {
    let is_full = ctx
        .store
        .get_device(device_id)
        .await?
        .map(|d| d.trusted && d.trust_level == TrustLevel::Full)
        .unwrap_or(false);
    if !is_full {
        return Ok(false);
    }
    let addr = resolve_addr(
        &ctx.store,
        &ctx.discovery,
        &ctx.identity,
        &ctx.fallback_name,
        &ctx.fallback_save_dir,
        device_id,
    )
    .await;

    let (items, via) = match ctx.transfer.fetch_index(device_id, addr).await {
        Ok(out) => out,
        Err(e) => {
            // 归类靠**结构性事实**（有没有解析出地址、本机开没开远程），不解析错误字符串。
            let remote_enabled = crate::settings::remote_enabled(&ctx.store).await;
            let why = reach::classify(&e, addr.is_some(), remote_enabled);
            reach::record(&ctx.reach, &ctx.events, device_id, Err(why), now_ms());
            return Err(e);
        }
    };
    let now = now_ms();
    reach::record(&ctx.reach, &ctx.events, device_id, Ok(via), now);
    let entries: Vec<RemoteIndexEntry> = items
        .into_iter()
        .map(|i| RemoteIndexEntry {
            device_id: device_id.clone(),
            rel_path: i.rel_path,
            size: i.size,
            hash: i.hash,
            seen_at: now,
        })
        .collect();
    ctx.store.replace_remote_index(device_id, entries).await?;
    let _ = ctx.events.send(CoreEvent::SyncIndexUpdated);
    Ok(true)
}

/// 对当前**全部完全信任配对设备**各尝试拉取一次（不再局限于 mDNS 在线快照，见模块文档；
/// 手动「刷新」与启动初拉、周期定时器共用）。
pub(crate) async fn refresh_all_full_trust(ctx: &ExchangeCtx) {
    let devices = match ctx.store.list_paired_devices().await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, "list paired devices failed");
            return;
        }
    };
    for dev in devices {
        if dev.trust_level != TrustLevel::Full {
            continue;
        }
        if let Err(e) = fetch_one(ctx, &dev.id).await {
            tracing::debug!(device = %dev.id, error = %e, "index fetch failed");
        }
    }
}

/// 启动后台交换循环：先全量初拉一次，之后 `DeviceFound` 即时触发 + 周期定时器兜底
/// （远程设备靠周期定时器，见模块文档）。
pub(crate) fn spawn_exchange_loop(ctx: ExchangeCtx, stop: CancellationToken) {
    let mut sub = ctx.events.subscribe();
    tokio::spawn(async move {
        refresh_all_full_trust(&ctx).await;
        let mut tick = tokio::time::interval(REMOTE_REFRESH_INTERVAL);
        tick.tick().await; // 首次 tick 立即完成，上面已经拉过一次，跳过
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                _ = tick.tick() => {
                    refresh_all_full_trust(&ctx).await;
                }
                msg = sub.recv() => {
                    match msg {
                        Ok(CoreEvent::DeviceFound(dev)) => {
                            if let Err(e) = fetch_one(&ctx, &dev.id).await {
                                tracing::debug!(device = %dev.id, error = %e, "index fetch on discovery failed");
                            }
                        }
                        Ok(_) => continue,
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            }
        }
    });
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
