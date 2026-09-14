//! 设备可达性状态（V0.8「Focus」F2）。
//!
//! 只存内存、只由**既有的**索引交换轮次喂数据——不新开后台循环。
//!
//! # 为什么搭在索引交换上
//!
//! `sync_exchange` 的周期轮次（30 秒，见该模块文档）本来就在做这件事的全部困难部分：
//! 遍历每台完全信任设备 → 走完整的连接阶梯 → 连上或失败。缺的只是「把结果记下来」。
//! 另开一条 ping 循环的话，两条循环会对同一批设备重复建连，而且很容易给出互相矛盾
//! 的答案（一条说连得上、另一条说连不上），那比没有更糟。
//!
//! **代价说清楚**：只覆盖**完全信任**设备。朋友级设备不参与索引交换，因此它们的
//! 可达性永远是 `Unknown`。这是有意的——朋友级本来就不参与跨设备索引与同步
//! （SYNC_DESIGN.md §2），为它们单独建连只为了在界面上画一个点，不值得。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aa4c_types::{
    ConnectionVia, CoreEvent, DeviceId, DeviceReachability, ReachFailure, ReachState,
};

use crate::EventSender;

/// 全部设备可达性的内存快照。廉价克隆（内部一个 `Arc`），同 `PortMapState` 的既有形态。
#[derive(Clone, Default)]
pub(crate) struct ReachState_(Arc<Mutex<HashMap<DeviceId, DeviceReachability>>>);

impl ReachState_ {
    /// 记录一次**成功**的探测。
    ///
    /// 返回 `true` 表示状态确实变了——调用方据此决定要不要发事件。每 30 秒一轮、
    /// 每轮都发的话界面会无谓地闪（同 `IntroductionsUpdated` 「只在确实新增时才发」
    /// 的既有取舍）。
    pub(crate) fn record_ok(&self, id: &DeviceId, via: ConnectionVia, now_ms: i64) -> bool {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let cur = map
            .entry(id.clone())
            .or_insert_with(|| DeviceReachability::unknown(id.clone()));
        let changed = cur.state != ReachState::Reachable || cur.via != Some(via);
        cur.state = ReachState::Reachable;
        cur.via = Some(via);
        cur.last_ok_at = Some(now_ms);
        cur.reason = None;
        changed
    }

    /// 记录一次**失败**的探测。`via` 与 `last_ok_at` 保留上一次成功时的值——
    /// 「上次是走中继连上的」对排查有用，界面按 `state` 决定弱化显示。
    pub(crate) fn record_fail(&self, id: &DeviceId, reason: ReachFailure) -> bool {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let cur = map
            .entry(id.clone())
            .or_insert_with(|| DeviceReachability::unknown(id.clone()));
        let changed = cur.state != ReachState::Unreachable || cur.reason != Some(reason);
        cur.state = ReachState::Unreachable;
        cur.reason = Some(reason);
        changed
    }

    /// 解除配对时清掉，免得一台已经不存在的设备继续挂在状态图上。
    pub(crate) fn forget(&self, id: &DeviceId) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    /// 当前快照。传入的是「现在有哪些已配对设备」——**没探测过的也要出现在结果里**
    /// （`Unknown`），否则界面会以为这台设备不存在。
    pub(crate) fn snapshot(&self, paired: &[DeviceId]) -> Vec<DeviceReachability> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        paired
            .iter()
            .map(|id| {
                map.get(id)
                    .cloned()
                    .unwrap_or_else(|| DeviceReachability::unknown(id.clone()))
            })
            .collect()
    }
}

/// 记录结果并在**状态确实变了**时广播一次。
pub(crate) fn record(
    state: &ReachState_,
    events: &EventSender,
    id: &DeviceId,
    outcome: Result<ConnectionVia, ReachFailure>,
    now_ms: i64,
) {
    let changed = match outcome {
        Ok(via) => state.record_ok(id, via, now_ms),
        Err(reason) => state.record_fail(id, reason),
    };
    if changed {
        let _ = events.send(CoreEvent::ReachabilityUpdated);
    }
}

/// 把一次探测失败归到哪一类——判据是**用户的下一步不同**，不是错误在代码里长什么样。
///
/// `had_addr` / `remote_enabled` 由调用方直接给出，**不去解析错误字符串**：
/// 「没解析出地址而且远程连接还关着」是个结构性的事实，`fetch_one` 手上就有，
/// 靠 match 我们自己拼出来的错误消息来倒推它，下次改一句文案就会悄悄失效。
pub(crate) fn classify(
    err: &aa4c_types::Aa4cError,
    had_addr: bool,
    remote_enabled: bool,
) -> ReachFailure {
    use aa4c_types::Aa4cError;
    if !had_addr && !remote_enabled {
        // 局域网里没看见它，而本机压根没开出网——没有任何一条路可走。
        return ReachFailure::NotOnLanAndRemoteOff;
    }
    match err {
        // 握手上了但对不上：版本太老不认识索引消息、或对端明确拒绝。
        Aa4cError::Protocol(_) | Aa4cError::NotPaired(_) => ReachFailure::PeerRefused,
        _ => ReachFailure::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ReachState_ {
        ReachState_::default()
    }

    /// 只在**状态确实变了**时才报「变了」——每 30 秒一轮、每轮都发事件的话界面会闪。
    #[test]
    fn repeating_the_same_outcome_is_not_a_change() {
        let s = state();
        let id: DeviceId = "d1".into();
        assert!(s.record_ok(&id, ConnectionVia::Lan, 1), "首次必然是变化");
        assert!(
            !s.record_ok(&id, ConnectionVia::Lan, 2),
            "同一档重复不算变化"
        );
        assert!(
            s.record_ok(&id, ConnectionVia::Relay, 3),
            "换了档位要算变化——用户会看到从「局域网」变成「中继」"
        );
        assert!(s.record_fail(&id, ReachFailure::Unreachable));
        assert!(!s.record_fail(&id, ReachFailure::Unreachable));
    }

    /// 失败之后仍保留上一次连上的档位与时刻——排查时「上次是怎么连上的」有用。
    #[test]
    fn a_failure_keeps_the_last_successful_rung_for_context() {
        let s = state();
        let id: DeviceId = "d1".into();
        s.record_ok(&id, ConnectionVia::PublicV6, 1_700_000_000_000);
        s.record_fail(&id, ReachFailure::Unreachable);

        let snap = s.snapshot(std::slice::from_ref(&id));
        assert_eq!(snap[0].state, ReachState::Unreachable);
        assert_eq!(snap[0].via, Some(ConnectionVia::PublicV6));
        assert_eq!(snap[0].last_ok_at, Some(1_700_000_000_000));
        assert_eq!(snap[0].reason, Some(ReachFailure::Unreachable));
    }

    /// 没探测过的设备也要出现在快照里，状态是 `Unknown`——漏掉的话界面会以为
    /// 这台设备不存在，而它其实只是还没轮到。
    #[test]
    fn devices_never_probed_still_appear_as_unknown() {
        let s = state();
        s.record_ok(&"d1".into(), ConnectionVia::Lan, 1);
        let snap = s.snapshot(&["d1".into(), "d2".into()]);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[1].device_id, "d2");
        assert_eq!(snap[1].state, ReachState::Unknown);
    }

    /// 解除配对之后不该继续挂在状态图上。
    #[test]
    fn forgetting_a_device_drops_its_state() {
        let s = state();
        let id: DeviceId = "d1".into();
        s.record_ok(&id, ConnectionVia::Lan, 1);
        s.forget(&id);
        assert_eq!(s.snapshot(&[id])[0].state, ReachState::Unknown);
    }

    /// 归类看的是**结构性事实**（有没有地址、开没开远程），不是错误消息的字面。
    #[test]
    fn classification_uses_facts_not_error_strings() {
        use aa4c_types::Aa4cError;
        let net = Aa4cError::Network("connection refused".into());
        let proto = Aa4cError::Protocol("peer proto 1 too old".into());

        // 局域网没看见 + 远程没开 = 根本没有路，与具体错误无关
        assert_eq!(
            classify(&net, false, false),
            ReachFailure::NotOnLanAndRemoteOff
        );
        assert_eq!(
            classify(&proto, false, false),
            ReachFailure::NotOnLanAndRemoteOff
        );
        // 有地址时才谈得上「试过了连不上」
        assert_eq!(classify(&net, true, false), ReachFailure::Unreachable);
        // 远程开着 = 有中继可试，即使没解析出地址也算试过
        assert_eq!(classify(&net, false, true), ReachFailure::Unreachable);
        // 协议对不上是另一回事：下一步是「确认两边都是新版本」
        assert_eq!(classify(&proto, true, true), ReachFailure::PeerRefused);
    }
}
