//! 双实例 mDNS 互发现集成测试。
//!
//! 依赖真实组播网络（CI 无组播环境），按 TESTING.md §3 标记 #[ignore]，
//! 本地手动运行：`cargo test -p aa4c-discovery -- --ignored`

use std::time::Duration;

use aa4c_discovery::DiscoveryService;
use aa4c_types::{CoreEvent, DeviceInfo, Platform};
use tokio::sync::broadcast;
use tokio::time::timeout;

fn make_info(seed: u8, name: &str) -> DeviceInfo {
    DeviceInfo {
        id: blake3::hash(&[seed; 32]).to_hex().to_string(),
        name: name.into(),
        platform: Platform::Macos,
        version: "0.1.0".into(),
        addr: None,
        online: true,
        trusted: false,
        trust_level: None,
    }
}

#[tokio::test]
#[ignore = "requires real multicast network"]
async fn two_instances_discover_each_other_and_detect_loss() {
    let info_a = make_info(1, "实例A");
    let info_b = make_info(2, "实例B");

    let (tx_a, mut rx_a) = broadcast::channel(64);
    let (tx_b, mut rx_b) = broadcast::channel(64);
    let a = DiscoveryService::new(info_a.clone(), tx_a).unwrap();
    let b = DiscoveryService::new(info_b.clone(), tx_b).unwrap();

    a.start(42421).await.unwrap();
    b.start(42422).await.unwrap();

    // A 应在 10 秒内发现 B（验收标准：10 秒内互相发现）
    let found_b = timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(CoreEvent::DeviceFound(d)) = rx_a.recv().await {
                if d.id == info_b.id {
                    return d;
                }
            }
        }
    })
    .await
    .expect("A should discover B within 10s");
    assert_eq!(found_b.name, "实例B");
    assert!(
        found_b.addr.is_some(),
        "discovered device should carry addr"
    );

    // B 也应发现 A
    timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(CoreEvent::DeviceFound(d)) = rx_b.recv().await {
                if d.id == info_a.id {
                    return;
                }
            }
        }
    })
    .await
    .expect("B should discover A within 10s");

    // 快照接口一致
    assert!(a.devices().iter().any(|d| d.id == info_b.id));

    // B 下线（注销发 goodbye 报文）→ A 收到 DeviceLost
    b.stop().await.unwrap();
    timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(CoreEvent::DeviceLost { id }) = rx_a.recv().await {
                if id == info_b.id {
                    return;
                }
            }
        }
    })
    .await
    .expect("A should see B go offline");
    assert!(!a.devices().iter().any(|d| d.id == info_b.id));

    a.stop().await.unwrap();
}
