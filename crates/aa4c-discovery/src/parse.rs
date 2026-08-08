//! mDNS TXT 记录 ↔ DeviceInfo 转换（PROTOCOL.md §1）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use aa4c_types::{DeviceInfo, Platform};
use mdns_sd::ServiceInfo;

/// TXT 记录键（PROTOCOL.md §1）。
pub(crate) const TXT_ID: &str = "id";
pub(crate) const TXT_NAME: &str = "name";
pub(crate) const TXT_PLATFORM: &str = "platform";
pub(crate) const TXT_VERSION: &str = "ver";
pub(crate) const TXT_PROTO: &str = "proto";

/// 解析已解析的 mDNS 服务为 DeviceInfo。
///
/// 任一必填字段缺失或非法 → None（按协议规则忽略该设备，不报错）。
/// `trusted` 由上层结合 devices 表填充，此处恒为 false。
pub(crate) fn parse_service(info: &ServiceInfo) -> Option<DeviceInfo> {
    let id = info.get_property_val_str(TXT_ID)?.to_string();
    // DeviceId = BLAKE3 hex，固定 64 字符
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let name = info.get_property_val_str(TXT_NAME)?.to_string();
    let platform: Platform = info.get_property_val_str(TXT_PLATFORM)?.parse().ok()?;
    let version = info.get_property_val_str(TXT_VERSION)?.to_string();
    let _proto: u16 = info.get_property_val_str(TXT_PROTO)?.parse().ok()?;

    let addr = pick_addr(info);
    Some(DeviceInfo {
        id,
        name,
        platform,
        version,
        addr,
        online: true,
        trusted: false,
        trust_level: None,
    })
}

/// 不可达地址的优先级（被排除，仅在无其他选择时才退而用之）。
const REJECT: u8 = 250;

/// 选择连接地址。
///
/// `enable_addr_auto` 会把对端**所有**网卡地址都广播出来，其中可能混入虚拟
/// 网卡的不可达地址（最典型：Clash/代理 TUN 模式默认 fake-ip 段 198.18.0.0/16）。
/// 这里按可达性打分，IPv4 与 IPv6 **在同一把尺子上比**（里程碑 R1，TRUST_DESIGN.md §6.1）。
///
/// 排序理由（越小越优先，见 [`addr_rank`]）：
/// 0. 私有 LAN IPv4 与 IPv6 ULA —— 同一个局域网内的直连，最快也最稳；
/// 1. **全局单播 IPv6** —— 同网段内同样是一跳直达，而且**离开这个局域网之后依然有效**，
///    所以排在"其他可路由 IPv4"前面。国内家宽普遍下发公网 IPv6，而 IPv4 反而在 CGNAT
///    后面（那一段直接被拒），这条排序正是让「公网直连」这一档真正能被命中的原因；
/// 2. 其他可路由 IPv4。
///
/// 一律排除：回环、链路本地（IPv6 的 `fe80::/10` 没有 scope id 根本连不上）、fake-ip、CGNAT。
fn pick_addr(info: &ServiceInfo) -> Option<SocketAddr> {
    let port = info.get_port();
    let addrs = info.get_addresses();

    let mut best: Option<(u8, IpAddr)> = None;
    for ip in addrs.iter() {
        let rank = addr_rank(ip);
        if rank < REJECT && best.map(|(r, _)| rank < r).unwrap_or(true) {
            best = Some((rank, *ip));
        }
    }
    if let Some((_, ip)) = best {
        return Some(SocketAddr::new(ip, port));
    }
    // 一个可达地址都没有：退回任意广播地址，总比没有强
    addrs.iter().next().map(|ip| SocketAddr::new(*ip, port))
}

/// 统一可达性优先级（越小越优先；`REJECT` 表示不可达）。
fn addr_rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V4(v4) => ipv4_rank(v4),
        IpAddr::V6(v6) => ipv6_rank(v6),
    }
}

/// IPv4 可达性优先级（越小越优先；`REJECT` 表示不可达）。
fn ipv4_rank(ip: &Ipv4Addr) -> u8 {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
        return REJECT; // 127/8、169.254/16、0.0.0.0
    }
    if a == 198 && (b == 18 || b == 19) {
        return REJECT; // 198.18.0.0/15 基准测试段：Clash 等代理 TUN 默认 fake-ip
    }
    if a == 100 && (64..=127).contains(&b) {
        return REJECT; // 100.64.0.0/10 运营商级 NAT
    }
    if ip.is_private() {
        return 0; // 192.168/16、10/8、172.16/12 —— 家用/办公 LAN，最优
    }
    2 // 其他可路由 IPv4：排在全局 IPv6 之后，见 `pick_addr` 的排序理由
}

/// IPv6 可达性优先级（越小越优先；`REJECT` 表示不可达）。
fn ipv6_rank(ip: &Ipv6Addr) -> u8 {
    if ip.is_loopback() || ip.is_unspecified() {
        return REJECT; // ::1、::
    }
    // 链路本地 fe80::/10：没有 scope id（`%en0`）就无法路由，而 mDNS 的 A/AAAA 记录
    // 里带不出 scope id——拿到也连不上，直接排除。
    if (ip.segments()[0] & 0xffc0) == 0xfe80 {
        return REJECT;
    }
    // IPv4 映射地址（::ffff:a.b.c.d）：按它内含的那个 IPv4 打分，别当成 IPv6 高看一眼。
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_rank(&v4);
    }
    // ULA fc00::/7：等价于 IPv4 的私有段——同一局域网内可用（mDNS 发现到的本来就是
    // 同网设备），离开这个网就不可路由。与私有 IPv4 同级。
    if (ip.segments()[0] & 0xfe00) == 0xfc00 {
        return 0;
    }
    // 全局单播 2000::/3：同网段一跳直达，且**跨网依然有效**——排在其他可路由 IPv4 之前。
    if (ip.segments()[0] & 0xe000) == 0x2000 {
        return 1;
    }
    REJECT // 其余（多播 ff00::/8、保留段等）
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_types::SERVICE_TYPE;

    fn make_service(props: &[(&str, &str)]) -> ServiceInfo {
        ServiceInfo::new(
            SERVICE_TYPE,
            "test-instance",
            "test-instance.local.",
            "192.168.1.5",
            42420,
            props,
        )
        .unwrap()
    }

    fn valid_props() -> Vec<(&'static str, String)> {
        vec![
            ("id", "ab".repeat(32)),
            ("name", "客厅电脑".into()),
            ("platform", "macos".into()),
            ("ver", "0.1.0".into()),
            ("proto", "1".into()),
        ]
    }

    #[test]
    fn parses_valid_txt_record() {
        let props = valid_props();
        let props_ref: Vec<(&str, &str)> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let device = parse_service(&make_service(&props_ref)).unwrap();
        assert_eq!(device.id, "ab".repeat(32));
        assert_eq!(device.name, "客厅电脑");
        assert_eq!(device.platform, Platform::Macos);
        assert_eq!(device.addr.unwrap().to_string(), "192.168.1.5:42420");
        assert!(device.online);
        assert!(!device.trusted);
    }

    #[test]
    fn ipv4_rank_prefers_lan_and_rejects_proxy() {
        // 私有 LAN 最优
        assert_eq!(ipv4_rank(&"192.168.1.5".parse().unwrap()), 0);
        assert_eq!(ipv4_rank(&"10.0.0.3".parse().unwrap()), 0);
        // Clash fake-ip / 回环 / 链路本地 / CGNAT 一律排除
        assert_eq!(ipv4_rank(&"198.18.0.1".parse().unwrap()), REJECT);
        assert_eq!(ipv4_rank(&"127.0.0.1".parse().unwrap()), REJECT);
        assert_eq!(ipv4_rank(&"169.254.1.2".parse().unwrap()), REJECT);
        assert_eq!(ipv4_rank(&"100.64.0.1".parse().unwrap()), REJECT);
        // 私有 LAN 优先级高于普通公网
        assert!(
            ipv4_rank(&"192.168.0.1".parse().unwrap()) < ipv4_rank(&"8.8.8.8".parse().unwrap())
        );
    }

    #[test]
    fn ipv6_rank_prefers_global_unicast_over_plain_public_ipv4() {
        let ula: Ipv6Addr = "fd12:3456::1".parse().unwrap();
        let global: Ipv6Addr = "2408:8000:1234::1".parse().unwrap();
        let link_local: Ipv6Addr = "fe80::1".parse().unwrap();

        // ULA 等价于私有 IPv4：同一局域网内可用，与私有 IPv4 同级
        assert_eq!(ipv6_rank(&ula), 0);
        assert_eq!(ipv6_rank(&ula), ipv4_rank(&"192.168.1.5".parse().unwrap()));

        // 链路本地没有 scope id 就连不上（mDNS 记录里带不出来），排除
        assert_eq!(ipv6_rank(&link_local), REJECT);
        assert_eq!(ipv6_rank(&"::1".parse().unwrap()), REJECT);
        assert_eq!(ipv6_rank(&"ff02::1".parse().unwrap()), REJECT);

        // 这一条是里程碑 R1 的核心主张：全局 IPv6 排在"其他可路由 IPv4"之前——
        // 前者跨网依然有效，后者出了这个网多半要打洞或中继。
        assert!(ipv6_rank(&global) < ipv4_rank(&"203.0.113.7".parse().unwrap()));
        // 但同局域网的私有 IPv4 仍然最优，不因为打通 IPv6 就把既有行为掀翻
        assert!(ipv4_rank(&"192.168.1.5".parse().unwrap()) < ipv6_rank(&global));

        // IPv4 映射地址按它内含的 IPv4 打分，不能因为"长得像 IPv6"就高看一眼
        assert_eq!(
            ipv6_rank(&"::ffff:100.64.0.1".parse().unwrap()),
            REJECT,
            "映射进来的 CGNAT 地址照样该拒"
        );
        assert_eq!(ipv6_rank(&"::ffff:192.168.1.5".parse().unwrap()), 0);
    }

    #[test]
    fn pick_addr_picks_global_ipv6_when_ipv4_is_behind_cgnat() {
        // 真实场景：国内家宽下发公网 IPv6，IPv4 落在运营商级 NAT 后面。
        // 旧实现只看 IPv4，CGNAT 被拒之后会退回"任意地址"，选中什么全看顺序；
        // 现在应当明确选中那个全局 IPv6。
        let addrs: Vec<IpAddr> = vec![
            "100.64.12.7".parse().unwrap(),       // CGNAT，不可达
            "2408:8000:1234::1".parse().unwrap(), // 公网 IPv6
        ];
        let best = addrs
            .iter()
            .filter(|ip| addr_rank(ip) < REJECT)
            .min_by_key(|ip| addr_rank(ip))
            .copied();
        assert_eq!(best, Some("2408:8000:1234::1".parse().unwrap()));
    }

    #[test]
    fn rejects_missing_or_invalid_fields() {
        // 缺 id
        let device = parse_service(&make_service(&[("name", "x")]));
        assert!(device.is_none());

        // id 非 64 位 hex
        let mut props = valid_props();
        props[0].1 = "not-hex".into();
        let props_ref: Vec<(&str, &str)> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(parse_service(&make_service(&props_ref)).is_none());

        // 非法 platform
        let mut props = valid_props();
        props[2].1 = "freebsd".into();
        let props_ref: Vec<(&str, &str)> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(parse_service(&make_service(&props_ref)).is_none());

        // 非法 proto
        let mut props = valid_props();
        props[4].1 = "abc".into();
        let props_ref: Vec<(&str, &str)> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(parse_service(&make_service(&props_ref)).is_none());
    }
}
