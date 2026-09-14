//! 设备可达性（V0.8「Focus」F2）：每台已配对设备**现在**能不能连上、走的哪一档。
//!
//! # 为什么需要它
//!
//! 连接阶梯（CONNECT_DESIGN.md §2）一直是算完就扔：`dial` 每次都判定出走的哪一档，
//! 报给一次传输任务之后就没了；`fetch_index` 干脆 `let (mut stream, _via) = ...`
//! 直接丢掉。于是界面上永远只有「在线 / 离线」两个状态——用户没法知道自己是在
//! 局域网里、还是真的跨网连上了、又或者只是在绕中继；连不上时更没法知道卡在哪。
//!
//! 而「让我的几台设备在任何网络下自己连成一片」正是 AA4C 唯一不可替代的能力。
//! 把它做出来却不让人看见，等于没做。
//!
//! # 不落库
//!
//! 可达性是**当下**的属性。重启之后「昨天它在局域网里」这种信息比没有更糟——
//! 会让用户以为现在也是。所以只存内存，重启后是 [`ReachState::Unknown`]，
//! 等第一轮探测（启动时就跑一次）填上。

use serde::{Deserialize, Serialize};

use crate::{ConnectionVia, DeviceId};

/// 一台设备当下的可达状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachState {
    /// 还没探测过（刚启动）。**不是「离线」**——界面上要说「正在检查」，
    /// 说成离线是在编造一个我们并不知道的事实。
    Unknown,
    /// 最近一次探测连上了。
    Reachable,
    /// 最近一次探测没连上。
    Unreachable,
}

/// 连不上的原因。**稳定的蛇形码，不是给人看的句子**——人话文案住在前端
/// `lib/format.ts`，同 `Aa4cError::code()` 与 UI_DESIGN_SPEC §6 的既有约定
/// （后端不写界面文案，也才有翻译的余地）。
///
/// 分这几档的判据是**用户的下一步不同**，不是错误在代码里长什么样：
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachFailure {
    /// 局域网里没发现它，而本机的远程连接是关着的——**根本没有路可走**。
    /// 下一步：把两台设备连到同一个网络，或者打开远程连接。
    NotOnLanAndRemoteOff,
    /// 有地址（或有中继可试）但没连上。对端离线、防火墙挡着、端口没放行都归这里。
    /// 下一步：确认对方开着、检查防火墙。
    Unreachable,
    /// 连上了，但对端拒绝或协议对不上（比如对端版本太老不认识索引交换）。
    /// 下一步：确认两边都是新版本。
    PeerRefused,
}

/// 一台设备的可达性快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceReachability {
    pub device_id: DeviceId,
    pub state: ReachState,
    /// 最近一次**连上时**走的档位。连不上时保留上一次的值——「上次是走中继连上的」
    /// 对排查有用，前端按 `state` 决定要不要弱化显示。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub via: Option<ConnectionVia>,
    /// 最近一次连上的时刻（unix 毫秒）。`None` = 本次启动以来一次都没连上。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_ok_at: Option<i64>,
    /// 仅在 [`ReachState::Unreachable`] 时有值。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<ReachFailure>,
}

impl DeviceReachability {
    /// 还没探测过的初始快照。
    pub fn unknown(device_id: DeviceId) -> Self {
        Self {
            device_id,
            state: ReachState::Unknown,
            via: None,
            last_ok_at: None,
            reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JSON 形状：camelCase 字段 + snake_case 取值，同本项目其余类型的既有约定。
    /// 空字段不出现在 JSON 里（`skip_serializing_if`），前端按可选处理。
    #[test]
    fn json_shape_matches_the_house_convention() {
        let r = DeviceReachability {
            device_id: "d1".into(),
            state: ReachState::Unreachable,
            via: Some(ConnectionVia::Relay),
            last_ok_at: Some(1_700_000_000_000),
            reason: Some(ReachFailure::NotOnLanAndRemoteOff),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["deviceId"], "d1");
        assert_eq!(json["state"], "unreachable");
        assert_eq!(json["via"], "relay");
        assert_eq!(json["lastOkAt"], 1_700_000_000_000i64);
        assert_eq!(json["reason"], "not_on_lan_and_remote_off");

        let back: DeviceReachability = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    /// 没探测过就是没探测过——`state` 必须是 `unknown`，不能是 `unreachable`。
    /// 界面据此说「正在检查」而不是「离线」：后者是在编造一个我们并不知道的事实。
    #[test]
    fn a_device_never_probed_is_unknown_not_unreachable() {
        let r = DeviceReachability::unknown("d1".into());
        assert_eq!(r.state, ReachState::Unknown);
        assert!(r.reason.is_none());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["state"], "unknown");
        // 空字段不该出现，免得前端把 null 当成一个值
        assert!(json.get("via").is_none());
        assert!(json.get("lastOkAt").is_none());
        assert!(json.get("reason").is_none());
    }
}
