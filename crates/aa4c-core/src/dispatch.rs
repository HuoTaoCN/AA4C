//! 配对分流适配器：把统一传输监听器读到的 `PairRequest` 接到 `PairingManager`。
//!
//! 传输层只认 [`IncomingPairDispatch`] trait，不感知配对语义；Core 在装配阶段
//! 注入本适配器，完成「传输监听 → 配对响应」的接线（AGENTS.md 低耦合约定）。

use std::sync::Arc;

use aa4c_identity::PairingManager;
use aa4c_proto::{write_message, IndexItem, Message};
use aa4c_store::Store;
use aa4c_transfer::{
    IncomingIndexDispatch, IncomingPairDispatch, IncomingTlsStream, ResolveFuture, ResolvedFetch,
    SharedFileResolver, SharedStream,
};
use aa4c_types::{DeviceId, DeviceInfo, TrustLevel};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// 写完最后一批 `IndexEntries` 后，索引交换协议没有任何后续的应答消息——不像
/// `Offer`/发送会话那样天然靠 `TaskDone`/`FileAck` 的最后一轮往返把连接"拖"到双方
/// 都确认完成。如果写完就直接返回，调用方（`IncomingIndexDispatch::dispatch`）持有的
/// `stream` 随之被丢弃，对 QUIC 承载来说这意味着底层连接立即被拆——而"写成功"只代表
/// 数据进了本地发送缓冲区，不代表已经送达对端；直接丢连接会把还没来得及发出的最后
/// 一批数据连同连接一起冲掉（实测踩到的真实竞态：QUIC 上稳定复现"connection lost"，
/// TCP 因为内核发送缓冲区的宽容度而不容易触发，这也是这个 bug 从里程碑 1 引入
/// QUIC 起就潜伏到现在才被里程碑 5 的打洞路径踩中的原因）。
///
/// 修法：显式半关闭写侧（`shutdown`，QUIC 下对应 `SendStream::finish`，确保排队的数据
/// 连同 FIN 一起被送出），再读到对端也关闭它那侧为止——两边都确认"数据交接完毕"后
/// 再真正丢弃连接。读到什么、读错什么都无所谓，只要这次读操作**完成**（无论是干净
/// EOF 还是对端直接重置），就说明连接层面已经有了明确结果，可以放心收尾了。
async fn finish_write_side(stream: &mut SharedStream) {
    let _ = stream.shutdown().await;
    let mut discard = [0u8; 1];
    let _ = stream.read(&mut discard).await;
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
