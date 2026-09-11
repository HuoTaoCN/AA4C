//! 配对分流适配器：把统一传输监听器读到的 `PairRequest` 接到 `PairingManager`。
//!
//! 传输层只认 [`IncomingPairDispatch`] trait，不感知配对语义；Core 在装配阶段
//! 注入本适配器，完成「传输监听 → 配对响应」的接线（AGENTS.md 低耦合约定）。

use std::sync::Arc;

use aa4c_identity::PairingManager;
use aa4c_proto::{write_message, IndexItem, Message};
use aa4c_store::Store;
use aa4c_transfer::{
    finish_write_side, IncomingIndexDispatch, IncomingPairDispatch, IncomingTlsStream,
    ResolveFuture, ResolvedFetch, ShareResolver, SharedFileResolver, SharedStream,
};
use aa4c_types::{DeviceId, DeviceInfo, TrustLevel};

use crate::unified;

/// 单批次最多条数（控制 `IndexEntries` 帧大小，远低于 16 MiB 帧上限）。
const INDEX_BATCH: usize = 1000;

pub(crate) struct PairDispatch {
    pairing: Arc<PairingManager>,
}

impl PairDispatch {
    pub(crate) fn new(pairing: Arc<PairingManager>) -> Self {
        Self { pairing }
    }
}

impl IncomingPairDispatch for PairDispatch {
    fn dispatch(
        &self,
        stream: IncomingTlsStream,
        cert_id: DeviceId,
        device: DeviceInfo,
        public_key: [u8; 32],
        proto: u16,
    ) {
        if let Err(e) = self
            .pairing
            .handle_dispatched(stream, cert_id, device, public_key, proto)
        {
            tracing::warn!(error = %e, "failed to dispatch incoming pairing");
        }
    }
}

/// 索引应答适配器：把统一监听器读到的 `IndexRequest` 接上本机共享索引（SYNC_DESIGN.md §3.3）。
///
/// 完全信任边界在此把关：只有 `trust_level = full` 的对端能拿到索引；否则回一个空批次
/// （不泄露任何文件名）。读取本机索引、按范围限定路径、分批回送全部在 Core 侧完成。
pub(crate) struct IndexServe {
    store: Store,
}

impl IndexServe {
    pub(crate) fn new(store: Store) -> Self {
        Self { store }
    }
}

impl IncomingIndexDispatch for IndexServe {
    fn dispatch(&self, stream: SharedStream, peer_id: DeviceId) {
        let store = self.store.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_index(store, stream, peer_id).await {
                tracing::warn!(error = %e, "failed to serve index request");
            }
        });
    }
}

async fn serve_index(
    store: Store,
    mut stream: SharedStream,
    peer_id: DeviceId,
) -> aa4c_types::Result<()> {
    // 完全信任过滤：非 full 设备一律回空批次
    let is_full = store
        .get_device(&peer_id)
        .await?
        .map(|d| d.trust_level == TrustLevel::Full)
        .unwrap_or(false);
    let items: Vec<IndexItem> = if is_full {
        unified::local_shared_items(&store).await?
    } else {
        Vec::new()
    };

    // 分批回送；空集合也发一个 last=true 的空批次表示「无共享」
    let mut chunks = items.chunks(INDEX_BATCH).peekable();
    if chunks.peek().is_none() {
        write_message(
            &mut stream,
            &Message::IndexEntries {
                entries: Vec::new(),
                last: true,
            },
        )
        .await?;
        finish_write_side(&mut stream).await;
        return Ok(());
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        write_message(
            &mut stream,
            &Message::IndexEntries {
                entries: chunk.to_vec(),
                last,
            },
        )
        .await?;
    }
    finish_write_side(&mut stream).await;
    Ok(())
}

/// 共享文件解析器：把拉取方请求的限定展示路径解析为本机共享文件（SYNC_DESIGN.md §4）。
///
/// 同样把守完全信任边界：非 `full` 对端一律 `None`（传输层据此回 `Cancel`）；
/// 路径解析只命中本机已索引（已对外广播）的条目，绝不按对端任意路径读盘。
pub(crate) struct FetchServe {
    store: Store,
}

impl FetchServe {
    pub(crate) fn new(store: Store) -> Self {
        Self { store }
    }
}

impl SharedFileResolver for FetchServe {
    fn resolve(&self, peer_id: DeviceId, rel_path: String) -> ResolveFuture {
        let store = self.store.clone();
        Box::pin(async move {
            let is_full = store
                .get_device(&peer_id)
                .await
                .ok()
                .flatten()
                .map(|d| d.trust_level == TrustLevel::Full)
                .unwrap_or(false);
            if !is_full {
                return None;
            }
            match unified::resolve_shared(&store, &rel_path).await {
                Ok(Some((abs, size))) => Some(ResolvedFetch {
                    abs,
                    rel_path,
                    size,
                }),
                _ => None,
            }
        })
    }
}

/// 分享 token 解析器：把打开分享链接的一方带来的 token 接到 `shares` 表校验 +
/// `resolve_shared` 路径解析（CONNECT_DESIGN.md §7，里程碑 C6）。
///
/// **不检查 `trusted`**——鉴权依据是 token 本身（有效、未过期、未吊销），不是配对关系
/// （见 [`aa4c_transfer::ShareResolver`] 文档）。解析成功即记一条 `share_access`（供
/// 「查看访问记录」，里程碑 C6 可选功能）；path 边界仍然复用 `resolve_shared`——分享的
/// 目标必须落在某个共享范围内，不会按任意路径读盘。
pub(crate) struct ShareServe {
    store: Store,
}

impl ShareServe {
    pub(crate) fn new(store: Store) -> Self {
        Self { store }
    }
}

impl ShareResolver for ShareServe {
    fn resolve(&self, token: String, peer_id: DeviceId) -> ResolveFuture {
        let store = self.store.clone();
        Box::pin(async move {
            let share = store.get_share_by_token(&token).await.ok().flatten()?;
            if share.status != "open" {
                return None;
            }
            if let Some(expires_at) = share.expires_at {
                if now_ms() >= expires_at {
                    return None;
                }
            }
            let (abs, size) = match unified::resolve_shared(&store, &share.rel_path).await {
                Ok(Some(v)) => v,
                _ => return None,
            };
            let _ = store
                .record_share_access(&share.id, Some(&peer_id), "download")
                .await;
            Some(ResolvedFetch {
                abs,
                rel_path: share.rel_path,
                size,
            })
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
