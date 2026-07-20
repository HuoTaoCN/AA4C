//! 配对协议端到端测试（V0.1_IMPLEMENTATION_PLAN.md M4）。
//!
//! 双实例（同进程、loopback TCP）：完整配对、拒绝路径、超时路径。

use std::sync::Arc;
use std::time::Duration;

use aa4c_identity::{Identity, PairingManager};
use aa4c_store::Store;
use aa4c_types::{CoreEvent, DeviceInfo, Platform};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

struct Node {
    manager: Arc<PairingManager>,
    store: Store,
    events: broadcast::Receiver<CoreEvent>,
    device: DeviceInfo,
    /// 监听地址（接收方角色）
    addr: std::net::SocketAddr,
    _dir: tempfile::TempDir,
}

/// 起一个节点：身份 + 库 + 配对管理器 + 入站监听循环。
async fn spawn_node(name: &str, session_timeout: Duration) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let identity = Arc::new(Identity::load_or_generate(dir.path()).unwrap());
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
    let (tx, rx) = broadcast::channel(64);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device = DeviceInfo {
        id: identity.device_id().clone(),
        name: name.into(),
        platform: Platform::Macos,
        version: "0.1.0".into(),
        addr: Some(addr),
        online: true,
        trusted: false,
        trust_level: None,
    };

    let manager = Arc::new(
        PairingManager::new(identity.clone(), device.clone(), store.clone(), tx)
            .with_timeout(session_timeout),
    );

    // 入站监听循环：TLS 握手后交给 PairingManager
    let acceptor = TlsAcceptor::from(Arc::new(identity.tls_server_config(None).unwrap()));
    let mgr = manager.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let mgr = mgr.clone();
            tokio::spawn(async move {
                if let Ok(stream) = acceptor.accept(tcp).await {
                    let _ = mgr.handle_incoming(stream);
                }
            });
        }
    });

    Node {
        manager,
        store,
        events: rx,
        device,
        addr,
        _dir: dir,
    }
}

/// 事件驱动器：模拟用户行为（接受请求 accept_request，PIN 确认 accept_pin），
/// 返回（本端看到的 PIN, PairingResult.success）。
async fn drive(
    manager: Arc<PairingManager>,
    mut events: broadcast::Receiver<CoreEvent>,
    accept_request: bool,
    accept_pin: bool,
) -> (Option<String>, bool) {
    let mut pin = None;
    loop {
        let event = events.recv().await.expect("event channel open");
        match event {
            CoreEvent::PairingRequest { session_id, .. } => {
                manager.confirm(&session_id, accept_request).await.unwrap();
            }
            CoreEvent::PairingPin { session_id, pin: p } => {
                pin = Some(p);
                manager.confirm(&session_id, accept_pin).await.unwrap();
            }
            CoreEvent::PairingResult { success, .. } => return (pin, success),
            _ => {}
        }
    }
}

#[tokio::test]
async fn full_pairing_succeeds_and_persists_both_sides() {
    let a = spawn_node("发起方A", Duration::from_secs(10)).await;
    let b = spawn_node("接收方B", Duration::from_secs(10)).await;

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    let (res_a, res_b) = tokio::join!(
        timeout(
            Duration::from_secs(10),
            drive(a.manager.clone(), a.events, true, true)
        ),
        timeout(
            Duration::from_secs(10),
            drive(b.manager.clone(), b.events, true, true)
        ),
    );
    let (pin_a, ok_a) = res_a.expect("A should finish in time");
    let (pin_b, ok_b) = res_b.expect("B should finish in time");

    assert!(ok_a && ok_b, "both sides should succeed");
    // PIN 两端独立计算且一致（6 位数字）
    let (pin_a, pin_b) = (pin_a.unwrap(), pin_b.unwrap());
    assert_eq!(pin_a, pin_b);
    assert_eq!(pin_a.len(), 6);

    // 双方都把对方写入 devices 表（trusted = 1）
    let paired_a = a.store.list_paired_devices().await.unwrap();
    let paired_b = b.store.list_paired_devices().await.unwrap();
    assert_eq!(paired_a.len(), 1);
    assert_eq!(paired_a[0].id, b.device.id);
    assert_eq!(paired_a[0].name, "接收方B");
    assert!(paired_a[0].paired_at.is_some());
    assert_eq!(paired_b.len(), 1);
    assert_eq!(paired_b[0].id, a.device.id);
}

/// PROTOCOL.md §17：配对时双向交换 `server_hint`（proto 5 起，两端都是新版本时的正常
/// 路径）。两节点各自在 store 里配好不同的 `server_url`/`enable_remote`，配对完成后断言
/// 双方 `devices.server_hint` 都拿到了对方声明的地址（对称验证两个方向）。
#[tokio::test]
async fn pairing_exchanges_server_hint_both_directions() {
    let a = spawn_node("发起方A", Duration::from_secs(10)).await;
    let b = spawn_node("接收方B", Duration::from_secs(10)).await;

    let server_a = "aa4c://server-a.example:42420#fpA";
    let server_b = "aa4c://server-b.example:42420#fpB";
    a.store
        .set_setting("server_url", &serde_json::to_string(server_a).unwrap())
        .await
        .unwrap();
    a.store
        .set_setting("enable_remote", &serde_json::to_string(&true).unwrap())
        .await
        .unwrap();
    b.store
        .set_setting("server_url", &serde_json::to_string(server_b).unwrap())
        .await
        .unwrap();
    b.store
        .set_setting("enable_remote", &serde_json::to_string(&true).unwrap())
        .await
        .unwrap();

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    let (res_a, res_b) = tokio::join!(
        timeout(
            Duration::from_secs(10),
            drive(a.manager.clone(), a.events, true, true)
        ),
        timeout(
            Duration::from_secs(10),
            drive(b.manager.clone(), b.events, true, true)
        ),
    );
    assert!(
        res_a.unwrap().1 && res_b.unwrap().1,
        "both sides should succeed"
    );

    let hint_a_about_b = a.store.get_device(&b.device.id).await.unwrap().unwrap();
    let hint_b_about_a = b.store.get_device(&a.device.id).await.unwrap().unwrap();
    assert_eq!(hint_a_about_b.server_hint.as_deref(), Some(server_b));
    assert_eq!(hint_b_about_a.server_hint.as_deref(), Some(server_a));
}

/// `enable_remote=false` 时对端不该声明 `server_hint`（即便 `server_url` 恰好写了值）——
/// 同 `orchestrate::share_link`/`remote_lookup` 的既有语义：`enable_remote` 才是总开关。
#[tokio::test]
async fn pairing_omits_server_hint_when_remote_disabled() {
    let a = spawn_node("发起方A", Duration::from_secs(10)).await;
    let b = spawn_node("接收方B", Duration::from_secs(10)).await;

    a.store
        .set_setting(
            "server_url",
            &serde_json::to_string("aa4c://server-a.example:42420#fpA").unwrap(),
        )
        .await
        .unwrap();
    // enable_remote 留空（未设置）——等同 false，同 settings::load 的默认语义。

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    let (res_a, res_b) = tokio::join!(
        timeout(
            Duration::from_secs(10),
            drive(a.manager.clone(), a.events, true, true)
        ),
        timeout(
            Duration::from_secs(10),
            drive(b.manager.clone(), b.events, true, true)
        ),
    );
    assert!(
        res_a.unwrap().1 && res_b.unwrap().1,
        "both sides should succeed"
    );

    let hint_b_about_a = b.store.get_device(&a.device.id).await.unwrap().unwrap();
    assert_eq!(hint_b_about_a.server_hint, None);
}

#[tokio::test]
async fn responder_rejecting_request_fails_both_sides() {
    let a = spawn_node("发起方A", Duration::from_secs(10)).await;
    let b = spawn_node("接收方B", Duration::from_secs(10)).await;

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    let (res_a, res_b) = tokio::join!(
        timeout(
            Duration::from_secs(10),
            drive(a.manager.clone(), a.events, true, true)
        ),
        // B 拒绝配对请求
        timeout(
            Duration::from_secs(10),
            drive(b.manager.clone(), b.events, false, true)
        ),
    );
    assert!(!res_a.unwrap().1, "A must fail when B rejects");
    assert!(!res_b.unwrap().1, "B reports failure too");

    assert!(a.store.list_paired_devices().await.unwrap().is_empty());
    assert!(b.store.list_paired_devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn pin_rejection_fails_pairing() {
    let a = spawn_node("发起方A", Duration::from_secs(10)).await;
    let b = spawn_node("接收方B", Duration::from_secs(10)).await;

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    let (res_a, res_b) = tokio::join!(
        // A 在 PIN 核对时点"不一致"
        timeout(
            Duration::from_secs(10),
            drive(a.manager.clone(), a.events, true, false)
        ),
        timeout(
            Duration::from_secs(10),
            drive(b.manager.clone(), b.events, true, true)
        ),
    );
    assert!(!res_a.unwrap().1);
    assert!(!res_b.unwrap().1);
    assert!(b.store.list_paired_devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn unanswered_request_times_out() {
    // 短超时：B 永不确认
    let a = spawn_node("发起方A", Duration::from_millis(800)).await;
    let b = spawn_node("接收方B", Duration::from_millis(800)).await;

    let mut peer_b = b.device.clone();
    peer_b.addr = Some(b.addr);
    a.manager.start_pairing(&peer_b).await.unwrap();

    // A 只等结果，不响应任何事件；B 完全不驱动（用户无操作）
    let mut events_a = a.events;
    let result = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::PairingResult { success, .. }) = events_a.recv().await {
                return success;
            }
        }
    })
    .await
    .expect("A should time out and report failure");
    assert!(!result, "timeout must yield failure");
}
