//! 跨设备索引交换（SYNC_DESIGN.md §3.3，里程碑 3）。
//!
//! 上线即与**在线的完全信任**设备交换索引摘要（只取元数据），落 `remote_index`，
//! 据此在统一视图里把对端文件标黄（在线可下载）/标红（离线）。按需拉取内容是里程碑 4。
//!
//! 触发策略（呼应里程碑 2 的扫描：定时 + 事件，不引入文件监听式复杂度）：
//! - 启动时对当前在线的完全信任设备各拉一次；
//! - 之后每当 `DeviceFound` 一台完全信任设备（上线/重连）就拉一次。
//!
//! 实时性足够，且避免对频繁的 `DeviceUpdated` 反复拉取。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aa4c_discovery::DiscoveryService;
use aa4c_store::Store;
use aa4c_transfer::TransferService;
use aa4c_types::{CoreEvent, DeviceInfo, RemoteIndexEntry, Result, TrustLevel};
use tokio::sync::broadcast::error::RecvError;

use crate::EventSender;

/// 与单台设备交换索引：仅对在线的完全信任设备生效，成功后广播 `SyncIndexUpdated`。
/// 返回 `true` 表示确实拉取并更新了远端索引。
pub(crate) async fn fetch_one(
    store: &Store,
    transfer: &Arc<TransferService>,
    events: &EventSender,
    dev: &DeviceInfo,
) -> Result<bool> {
    let is_full = store
        .get_device(&dev.id)
        .await?
        .map(|d| d.trusted && d.trust_level == TrustLevel::Full)
        .unwrap_or(false);
    let Some(addr) = dev.addr else {
        return Ok(false);
    };
    if !is_full {
        return Ok(false);
    }

    let items = transfer.fetch_index(&dev.id, addr).await?;
    let now = now_ms();
    let entries: Vec<RemoteIndexEntry> = items
        .into_iter()
        .map(|i| RemoteIndexEntry {
            device_id: dev.id.clone(),
            rel_path: i.rel_path,
            size: i.size,
            hash: i.hash,
            seen_at: now,
        })
        .collect();
    store.replace_remote_index(&dev.id, entries).await?;
    let _ = events.send(CoreEvent::SyncIndexUpdated);
    Ok(true)
}

/// 对当前所有在线设备各尝试拉取一次（手动「刷新」与启动初拉共用）。
pub(crate) async fn refresh_online(
    store: &Store,
    transfer: &Arc<TransferService>,
    discovery: &Arc<DiscoveryService>,
    events: &EventSender,
) {
    for dev in discovery.devices() {
        if let Err(e) = fetch_one(store, transfer, events, &dev).await {
            tracing::debug!(device = %dev.id, error = %e, "index fetch failed");
        }
    }
}

/// 启动后台交换循环：先对在线完全信任设备初拉一次，之后每逢 `DeviceFound` 再拉。
pub(crate) fn spawn_exchange_loop(
    store: Store,
    transfer: Arc<TransferService>,
    discovery: Arc<DiscoveryService>,
    events: EventSender,
) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        refresh_online(&store, &transfer, &discovery, &events).await;
        loop {
            match sub.recv().await {
                Ok(CoreEvent::DeviceFound(dev)) => {
                    if let Err(e) = fetch_one(&store, &transfer, &events, &dev).await {
                        tracing::debug!(device = %dev.id, error = %e, "index fetch on discovery failed");
                    }
                }
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
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
