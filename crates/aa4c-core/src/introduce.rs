//! 信任传递 / 引荐（TRUST_DESIGN.md §5，PROTOCOL.md §18，V0.7 里程碑 R2）。
//!
//! 解决的是一个死结：家里的台式机和单位的台式机永远不会同处一个局域网，而配对要走 PIN
//! （要求同网），于是它们连得上也互不认识。手机两边都去过，由它把「这也是你的设备」的
//! **指纹**捎过去，用户各点一次确认即可互信——不需要把台式机搬来搬去。
//!
//! 三条设计约束（详见 TRUST_DESIGN.md §5.2/§5.9）：
//! - **引荐 ≠ 信任**。这里只会落一条「待确认」记录，升级信任必须由用户在界面上点确认。
//!   刻意不做 Syncthing 式的自动引荐——它有官方文档自己记录的两个坑：传递失控、
//!   「删了又被加回来」。
//! - **只在 `full ↔ full` 之间交换，且只引荐 `full`**。`friend` 是「别人的设备」，
//!   把它广播出去等于泄露社交关系图。
//! - **收到的每一条都要自校验**：`device_id == BLAKE3(public_key)`（见 [`verify_intro`]）。
//!
//! 交换时机与索引交换（[`crate::sync_exchange`]）分开：那边 30s 一轮是因为文件随时在变，
//! 而「我有哪几台设备」几乎不变，没必要每 30s 就为它多开一条 TCP+TLS 连接。这里是
//! 启动时一轮 + [`INTRODUCE_INTERVAL`] 兜底。

use std::sync::Arc;
use std::time::Duration;

use aa4c_discovery::DiscoveryService;
use aa4c_identity::{device_id_from_public_key, Identity};
use aa4c_proto::PeerIntro;
use aa4c_store::{DeviceRecord, Store};
use aa4c_transfer::TransferService;
use aa4c_types::{CoreEvent, DeviceId, Platform, Result, TrustLevel};
use tokio_util::sync::CancellationToken;

use crate::orchestrate::resolve_addr;
use crate::EventSender;

/// 周期引荐交换的间隔。比索引交换（30s）慢得多：引荐内容几乎不变，而用户从「在单位配对
/// 完手机」到「走回家里那台电脑点确认」中间隔着的时间远不止 5 分钟，把它压到秒级毫无收益，
/// 只会给每台完全信任设备平白多出一条周期连接。
const INTRODUCE_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// 本机待确认列表的容量上限（TRUST_DESIGN.md §5.9「拒绝服务」）。超出后本轮新条目直接
/// 丢弃——一台被攻破的完全信任设备最廉价的骚扰方式就是刷满这个列表。
const MAX_PENDING: i64 = 200;

/// 校验一条引荐条目自洽：`device_id` 必须确实等于 `BLAKE3(public_key)`。
///
/// 这是引荐相对「当面配对」不掉安全性的关键一环。AA4C 的信任锚点本来就是证书指纹
/// （`device_id_from_cert`），引荐传的就是指纹本身，没有 TOFU 窗口；而把公钥一并带上，
/// 收方本地就能确认这两者对得上——恶意引荐者没法递来一个与公钥对不上的指纹。
fn verify_intro(intro: &PeerIntro) -> bool {
    intro.public_key.len() == 32 && device_id_from_public_key(&intro.public_key) == intro.device_id
}

/// 与单台完全信任设备交换一轮引荐；返回本轮是否**新增**了待确认记录。
pub(crate) async fn fetch_one(
    store: &Store,
    discovery: &DiscoveryService,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    transfer: &Arc<TransferService>,
    device_id: &DeviceId,
) -> Result<bool> {
    let is_full = store
        .get_device(device_id)
        .await?
        .is_some_and(|d| d.trusted && d.trust_level == TrustLevel::Full);
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

    let peers = transfer.fetch_introductions(device_id, addr).await?;
    let mut added = false;
    for intro in peers {
        // 自己被引荐回来是正常的（对端当然认识本机），静默跳过。
        if &intro.device_id == identity.device_id() {
            continue;
        }
        if !verify_intro(&intro) {
            tracing::warn!(
                introducer = %device_id,
                claimed = %intro.device_id,
                "introduction rejected: device_id does not match its public key"
            );
            continue;
        }
        let Ok(platform) = intro.platform.parse::<Platform>() else {
            tracing::debug!(platform = %intro.platform, "introduction with unknown platform");
            continue;
        };
        if store.count_pending_introductions().await? >= MAX_PENDING {
            tracing::warn!(introducer = %device_id, "pending introduction list is full");
            break;
        }
        let record = DeviceRecord {
            id: intro.device_id,
            name: intro.name,
            platform,
            public_key: intro.public_key,
            // 以下字段对「待确认」记录无意义，`record_introduction` 不会写它们
            // （它只写 id/name/platform/public_key/server_hint/introduced_by）。
            trusted: false,
            trust_level: TrustLevel::Friend,
            paired_at: None,
            last_seen_at: None,
            last_addr: None,
            server_hint: intro.server_hint,
            created_at: 0,
            updated_at: 0,
        };
        added |= store.record_introduction(&record, device_id).await?;
    }
    Ok(added)
}

/// 对当前全部完全信任设备各交换一轮引荐。只在确实新增了待确认记录时广播事件。
pub(crate) async fn refresh_all(
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
    let mut added = false;
    for dev in devices {
        if dev.trust_level != TrustLevel::Full {
            continue;
        }
        match fetch_one(
            store,
            discovery,
            identity,
            fallback_name,
            fallback_save_dir,
            transfer,
            &dev.id,
        )
        .await
        {
            Ok(new) => added |= new,
            // 对端版本太旧 / 不在线都会走到这里，是常态，不是错误。
            Err(e) => tracing::debug!(device = %dev.id, error = %e, "introduce exchange failed"),
        }
    }
    if added {
        let _ = events.send(CoreEvent::IntroductionsUpdated);
    }
}

/// 启动后台引荐循环：先全量一轮，之后按 [`INTRODUCE_INTERVAL`] 兜底。
///
/// 不订阅 `DeviceFound`：索引交换那边订阅是因为设备一上线就该看到它的新文件；引荐没有
/// 这个即时性诉求，而每次 mDNS 抖动都多开一条连接是实打实的开销。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_introduce_loop(
    store: Store,
    discovery: Arc<DiscoveryService>,
    identity: Arc<Identity>,
    fallback_name: String,
    fallback_save_dir: String,
    transfer: Arc<TransferService>,
    events: EventSender,
    stop: CancellationToken,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(INTRODUCE_INTERVAL);
        loop {
            // 首次 tick 立即完成 → 启动即跑一轮
            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                _ = tick.tick() => {}
            }
            refresh_all(
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intro(device_id: &str, public_key: Vec<u8>) -> PeerIntro {
        PeerIntro {
            device_id: device_id.into(),
            public_key,
            name: "单位电脑".into(),
            platform: "macos".into(),
            server_hint: None,
        }
    }

    #[test]
    fn accepts_a_self_consistent_introduction() {
        let key = vec![9u8; 32];
        let id = device_id_from_public_key(&key);
        assert!(verify_intro(&intro(&id, key)));
    }

    #[test]
    fn rejects_a_fingerprint_that_does_not_match_its_key() {
        // 恶意引荐者递来别人的公钥、配上自己想让你信任的指纹。
        let key = vec![9u8; 32];
        assert!(!verify_intro(&intro(&"a".repeat(64), key)));
    }

    #[test]
    fn rejects_a_key_of_the_wrong_length() {
        let key = vec![9u8; 31];
        let id = device_id_from_public_key(&key);
        assert!(
            !verify_intro(&intro(&id, key)),
            "非 32 字节不是 Ed25519 公钥"
        );
    }
}
