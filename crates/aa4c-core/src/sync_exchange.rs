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

use crate::orchestrate::resolve_addr;
use crate::EventSender;

/// 周期性全量刷新的间隔：`DeviceFound` 覆盖不到远程设备，靠这个兜底发现/重连
/// （个人自托管场景，几台设备的轮询开销可忽略；不与 `aa4c_server::REGISTER_TTL` 绑定，
/// 两者关注点不同——那是"我还在不在线"，这是"我要不要去问问对方"）。
const REMOTE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// 与单台设备交换索引：仅对完全信任设备生效，成功后广播 `SyncIndexUpdated`。
/// 返回 `true` 表示确实拉取并更新了远端索引。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_one(
    store: &Store,
    discovery: &DiscoveryService,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    transfer: &Arc<TransferService>,
    events: &EventSender,
    device_id: &DeviceId,
) -> Result<bool> {
    let is_full = store
        .get_device(device_id)
        .await?
        .map(|d| d.trusted && d.trust_level == TrustLevel::Full)
        .unwrap_or(false);
    if !is_full {
        return Ok(false);
    }
    let addr = resolve_addr(
        store,
        discovery,
        identity,
        fallback_name,
        fallback_save_dir,
        device_id,
    )
    .await;

    let items = transfer.fetch_index(device_id, addr).await?;
    let now = now_ms();
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
    store.replace_remote_index(device_id, entries).await?;
    let _ = events.send(CoreEvent::SyncIndexUpdated);
    Ok(true)
}

/// 对当前**全部完全信任配对设备**各尝试拉取一次（不再局限于 mDNS 在线快照，见模块文档；
/// 手动「刷新」与启动初拉、周期定时器共用）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refresh_all_full_trust(
    store: &Store,
    discovery: &DiscoveryService,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    transfer: &Arc<TransferService>,
    events: &EventSender,
) {
    let devices = match store.list_paired_devices().await {
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
        if let Err(e) = fetch_one(
            store,
            discovery,
            identity,
            fallback_name,
            fallback_save_dir,
            transfer,
            events,
            &dev.id,
        )
        .await
        {
            tracing::debug!(device = %dev.id, error = %e, "index fetch failed");
        }
    }
}

/// 启动后台交换循环：先全量初拉一次，之后 `DeviceFound` 即时触发 + 周期定时器兜底
/// （远程设备靠周期定时器，见模块文档）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_exchange_loop(
    store: Store,
    discovery: Arc<DiscoveryService>,
    identity: Arc<Identity>,
    fallback_name: String,
    fallback_save_dir: String,
    transfer: Arc<TransferService>,
    events: EventSender,
) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        refresh_all_full_trust(
            &store,
            &discovery,
            &identity,
            &fallback_name,
            &fallback_save_dir,
            &transfer,
            &events,
        )
        .await;
        let mut tick = tokio::time::interval(REMOTE_REFRESH_INTERVAL);
        tick.tick().await; // 首次 tick 立即完成，上面已经拉过一次，跳过
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    refresh_all_full_trust(
                        &store,
                        &discovery,
                        &identity,
                        &fallback_name,
                        &fallback_save_dir,
                        &transfer,
                        &events,
                    )
                    .await;
                }
                msg = sub.recv() => {
                    match msg {
                        Ok(CoreEvent::DeviceFound(dev)) => {
                            if let Err(e) = fetch_one(
                                &store,
                                &discovery,
                                &identity,
                                &fallback_name,
                                &fallback_save_dir,
                                &transfer,
                                &events,
                                &dev.id,
                            )
                            .await
                            {
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
