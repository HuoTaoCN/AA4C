//! mDNS TXT 记录 ↔ DeviceInfo 转换（PROTOCOL.md §1）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
    })
}

/// 不可达地址的优先级（被排除，仅在无其他选择时才退而用之）。
const REJECT: u8 = 250;

/// 选择连接地址。
///
/// `enable_addr_auto` 会把对端**所有**网卡地址都广播出来，其中可能混入虚拟
/// 网卡的不可达地址（最典型：Clash/代理 TUN 模式默认 fake-ip 段 198.18.0.0/16）。
/// 这里按可达性打分：私有 LAN IPv4 最优，排除回环 / 链路本地 / fake-ip / CGNAT。
fn pick_addr(info: &ServiceInfo) -> Option<SocketAddr> {
    let port = info.get_port();
    let addrs = info.get_addresses();

    let mut best: Option<(u8, Ipv4Addr)> = None;
    for ip in addrs.iter() {
        if let IpAddr::V4(v4) = ip {
            let rank = ipv4_rank(v4);
            if rank < REJECT && best.map(|(r, _)| rank < r).unwrap_or(true) {
                best = Some((rank, *v4));
            }
        }
    }
    if let Some((_, ip)) = best {
        return Some(SocketAddr::new(IpAddr::V4(ip), port));
    }
    // 没有可用 IPv4：退回任意广播地址（IPv6 等），总比没有强
    addrs.iter().next().map(|ip| SocketAddr::new(*ip, port))
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
    1 // 其他可路由 IPv4
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
