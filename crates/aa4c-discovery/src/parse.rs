//! mDNS TXT 记录 ↔ DeviceInfo 转换（PROTOCOL.md §1）。

use std::net::{IpAddr, SocketAddr};

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

/// 选择连接地址：优先 IPv4。
fn pick_addr(info: &ServiceInfo) -> Option<SocketAddr> {
    let addrs = info.get_addresses();
    let ip: IpAddr = addrs
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| addrs.iter().next())
        .copied()?;
    Some(SocketAddr::new(ip, info.get_port()))
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
