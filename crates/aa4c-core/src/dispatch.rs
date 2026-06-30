//! 配对分流适配器：把统一传输监听器读到的 `PairRequest` 接到 `PairingManager`。
//!
//! 传输层只认 [`IncomingPairDispatch`] trait，不感知配对语义；Core 在装配阶段
//! 注入本适配器，完成「传输监听 → 配对响应」的接线（AGENTS.md 低耦合约定）。

use std::sync::Arc;

use aa4c_identity::PairingManager;
use aa4c_proto::{write_message, IndexItem, Message};
use aa4c_store::Store;
use aa4c_transfer::{IncomingIndexDispatch, IncomingPairDispatch, IncomingTlsStream};
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
    ) {
        if let Err(e) = self
            .pairing
            .handle_dispatched(stream, cert_id, device, public_key)
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
    fn dispatch(&self, stream: IncomingTlsStream, peer_id: DeviceId) {
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
    mut stream: IncomingTlsStream,
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
    Ok(())
}
