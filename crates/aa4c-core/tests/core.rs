//! Core 端到端冒烟测试（V0.1_IMPLEMENTATION_PLAN.md M6）。
//!
//! 两个装配完整的 `Core`（loopback、ephemeral 端口、独立数据目录）：
//! 配对 → 传文件 → 断言双方记录落库。重点验证 M6 的统一监听器分流——
//! 配对连接打到对端「传输监听器」，由注入的钩子转交 `PairingManager`。
//!
//! 不依赖 mDNS（CI 无组播）：手工用真实监听端口构造对端地址，配对走
//! `core.pairing`，发送走 `core.send_files`（用配对落库的 last_addr 解析）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aa4c_core::{Core, CoreConfig};
use aa4c_types::{CoreEvent, DeviceInfo, TransferStatus};
use tokio::sync::broadcast;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);

struct Node {
    core: Arc<Core>,
    _dir: tempfile::TempDir,
}

async fn spawn_node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0; // ephemeral，避免双实例端口冲突
                            // 不用真实下载目录：否则启动时的 Inbox 初始扫描会扫到测试机的真实文件
    config.transfer.default_save_dir = dir.path().join("downloads");
    let core = Core::start(config).await.expect("core starts");
    Node { core, _dir: dir }
}

/// 同 [`spawn_node`]，但出站连接强制走 QUIC（里程碑 C1 的测试专用开关，
/// 见 `TransferConfig::prefer_quic`）。`start_listener` 总会 best-effort 绑定 QUIC，
/// 所以只需给发起方设这个开关，接收方按普通节点起即可同时监听 TCP 与 QUIC。
async fn spawn_node_quic() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    config.transfer.prefer_quic = true;
    let core = Core::start(config).await.expect("core starts");
    Node { core, _dir: dir }
}

/// 用真实监听端口构造 loopback 对端地址。
fn peer_info(node: &Node) -> DeviceInfo {
    DeviceInfo {
        addr: Some(
            format!("127.0.0.1:{}", node.core.listen_port())
                .parse()
                .unwrap(),
        ),
        ..node.core.self_info()
    }
}

/// 模拟用户：接受配对请求并确认 PIN。返回是否成功。
async fn drive_pairing(core: Arc<Core>, mut events: broadcast::Receiver<CoreEvent>) -> bool {
    loop {
        match events.recv().await.expect("event bus open") {
            CoreEvent::PairingRequest { session_id, .. } => {
                core.confirm_pairing(&session_id, true).await.unwrap();
            }
            CoreEvent::PairingPin { session_id, .. } => {
                core.confirm_pairing(&session_id, true).await.unwrap();
            }
            CoreEvent::PairingResult { success, .. } => return success,
            _ => {}
        }
    }
}

#[tokio::test]
async fn two_cores_pair_then_transfer() {
    let a = spawn_node().await;
    let b = spawn_node().await;
    let a_id = a.core.self_info().id;
    let b_id = b.core.self_info().id;

    // —— 配对：a 发起，连接打到 b 的传输监听器并分流到配对响应 ——
    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();

    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    // 双方都把对方写入 devices 表（trusted = 1）
    assert_eq!(a.core.store.list_paired_devices().await.unwrap().len(), 1);
    assert_eq!(b.core.store.list_paired_devices().await.unwrap().len(), 1);
    // a 的设备列表里 b 标记为已配对
    let listed = a.core.list_devices().await.unwrap();
    assert!(listed.iter().any(|d| d.id == b_id && d.trusted));

    // —— 传输：a 发文件给 b（send_files 用配对落库的 last_addr 解析对端）——
    let recv_dir = b._dir.path().join("inbox");
    let src = a._dir.path().join("hello.txt");
    tokio::fs::write(&src, b"AA4C M6 smoke").await.unwrap();

    // b 自动接受到指定目录
    let ev_b2 = b.core.subscribe();
    let b_core = b.core.clone();
    let recv_dir2 = recv_dir.clone();
    tokio::spawn(async move {
        let mut rx = ev_b2;
        while let Ok(event) = rx.recv().await {
            if let CoreEvent::TransferRequest { task } = event {
                b_core
                    .accept_transfer(&task.id, true, Some(recv_dir2.clone()))
                    .await
                    .unwrap();
            }
        }
    });

    let mut ev_a2 = a.core.subscribe();
    let task_id = a.core.send_files(&b_id, vec![src]).await.unwrap();

    let done = timeout(WAIT, async {
        loop {
            match ev_a2.recv().await.unwrap() {
                CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
                CoreEvent::TransferFailed { task_id: t, error } if t == task_id => {
                    panic!("transfer failed: {error}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("transfer reaches terminal state");
    assert!(done);

    // 文件落盘且内容正确
    let received: PathBuf = recv_dir.join("hello.txt");
    assert_eq!(tokio::fs::read(&received).await.unwrap(), b"AA4C M6 smoke");

    // 双方任务记录均为 done
    let a_tasks = a.core.list_transfers(10, 0).await.unwrap();
    let b_tasks = b.core.list_transfers(10, 0).await.unwrap();
    assert_eq!(a_tasks.len(), 1);
    assert_eq!(a_tasks[0].status, TransferStatus::Done);
    assert_eq!(a_tasks[0].peer, b_id);
    assert_eq!(b_tasks.len(), 1);
    assert_eq!(b_tasks[0].status, TransferStatus::Done);
    assert_eq!(b_tasks[0].peer, a_id);

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

#[tokio::test]
async fn index_exchange_gated_by_full_trust() {
    use aa4c_types::TrustLevel;

    let a = spawn_node().await;
    let b = spawn_node().await;
    let a_id = a.core.self_info().id;

    // —— 先配对（默认 friend）——
    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    // —— B 添加一个共享文件夹（含一个文件）并扫描入索引 ——
    let shared = b._dir.path().join("shared");
    tokio::fs::create_dir_all(&shared).await.unwrap();
    tokio::fs::write(shared.join("doc.txt"), b"hi")
        .await
        .unwrap();
    b.core.add_sync_scope(shared.clone()).await.unwrap();
    b.core.rescan_sync().await.unwrap();

    let b_addr = peer_info(&b).addr.unwrap();

    // —— A 仍是 B 眼中的 friend：B 拒绝交出索引（回空批次，不泄露任何文件名）——
    let items = a
        .core
        .transfer
        .fetch_index(&b.core.self_info().id, b_addr)
        .await
        .unwrap();
    assert!(items.is_empty(), "friend must not receive any shared index");

    // —— B 把 A 升级为「我的设备」（full）后，A 才能取到 B 的共享索引 ——
    b.core
        .set_trust_level(&a_id, TrustLevel::Full)
        .await
        .unwrap();
    let items = a
        .core
        .transfer
        .fetch_index(&b.core.self_info().id, b_addr)
        .await
        .unwrap();
    assert_eq!(items.len(), 1, "full device receives the shared file");
    // 限定路径：顶层段是共享文件夹名（last path segment）
    assert_eq!(items[0].rel_path, "shared/doc.txt");
    assert_eq!(items[0].size, 2);

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

/// 等待某任务到达终态：返回 true=完成(Done)，false=失败(Failed)。
async fn wait_terminal(mut events: broadcast::Receiver<CoreEvent>, task_id: &str) -> bool {
    loop {
        match events.recv().await.expect("event bus open") {
            CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
            CoreEvent::TransferFailed { task_id: t, .. } if t == task_id => return false,
            _ => {}
        }
    }
}

#[tokio::test]
async fn on_demand_fetch_pulls_file_and_gates_on_full_trust() {
    use aa4c_types::TrustLevel;

    let a = spawn_node().await;
    let b = spawn_node().await;
    let a_id = a.core.self_info().id;

    // 配对（默认 friend）
    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    // B 共享一个文件夹（含一个文件），扫描入索引
    let shared = b._dir.path().join("shared");
    tokio::fs::create_dir_all(&shared).await.unwrap();
    tokio::fs::write(shared.join("doc.txt"), b"pull me over ATP!")
        .await
        .unwrap();
    b.core.add_sync_scope(shared.clone()).await.unwrap();
    b.core.rescan_sync().await.unwrap();

    let dest = a._dir.path().join("pulled");

    // —— A 还是 friend：拉取被拒（B 的解析器回 None → Cancel → A 任务失败）——
    let ev = a.core.subscribe();
    let task = a
        .core
        .transfer
        .fetch_file(&peer_info(&b), "shared/doc.txt", Some(dest.clone()))
        .await
        .unwrap();
    assert!(
        !timeout(WAIT, wait_terminal(ev, &task)).await.unwrap(),
        "friend must not be able to pull"
    );
    assert!(
        !dest.join("doc.txt").exists(),
        "no file should land on refusal"
    );

    // —— B 把 A 升级为「我的设备」(full) 后，拉取成功，内容落盘 ——
    b.core
        .set_trust_level(&a_id, TrustLevel::Full)
        .await
        .unwrap();
    let ev = a.core.subscribe();
    let task = a
        .core
        .transfer
        .fetch_file(&peer_info(&b), "shared/doc.txt", Some(dest.clone()))
        .await
        .unwrap();
    assert!(
        timeout(WAIT, wait_terminal(ev, &task)).await.unwrap(),
        "full device pulls successfully"
    );
    // 落盘相对路径剥掉了顶层分组段「shared」
    assert_eq!(
        tokio::fs::read(dest.join("doc.txt")).await.unwrap(),
        b"pull me over ATP!"
    );

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

#[tokio::test]
async fn quic_roundtrip_transfer() {
    // A 出站连接强制走 QUIC（里程碑 C1，CONNECT_DESIGN.md §5）；B 照常起（TCP+QUIC
    // 都在监听，QUIC 用同一端口号，见 aa4c-transfer::quic::listen）。
    let a = spawn_node_quic().await;
    let b = spawn_node().await;
    let a_id = a.core.self_info().id;
    let b_id = b.core.self_info().id;

    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    let recv_dir = b._dir.path().join("inbox");
    let src = a._dir.path().join("hello.txt");
    tokio::fs::write(&src, b"AA4C over QUIC").await.unwrap();

    let ev_b2 = b.core.subscribe();
    let b_core = b.core.clone();
    let recv_dir2 = recv_dir.clone();
    tokio::spawn(async move {
        let mut rx = ev_b2;
        while let Ok(event) = rx.recv().await {
            if let CoreEvent::TransferRequest { task } = event {
                b_core
                    .accept_transfer(&task.id, true, Some(recv_dir2.clone()))
                    .await
                    .unwrap();
            }
        }
    });

    let mut ev_a2 = a.core.subscribe();
    let task_id = a.core.send_files(&b_id, vec![src]).await.unwrap();

    let done = timeout(WAIT, async {
        loop {
            match ev_a2.recv().await.unwrap() {
                CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
                CoreEvent::TransferFailed { task_id: t, error } if t == task_id => {
                    panic!("transfer failed: {error}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("transfer reaches terminal state");
    assert!(done);

    assert_eq!(
        tokio::fs::read(recv_dir.join("hello.txt")).await.unwrap(),
        b"AA4C over QUIC"
    );
    let a_tasks = a.core.list_transfers(10, 0).await.unwrap();
    assert_eq!(a_tasks[0].status, TransferStatus::Done);
    assert_eq!(a_tasks[0].peer, b_id);
    let b_tasks = b.core.list_transfers(10, 0).await.unwrap();
    assert_eq!(b_tasks[0].status, TransferStatus::Done);
    assert_eq!(b_tasks[0].peer, a_id);

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

/// 在 A→B 方向转发超过 `cut_after` 字节后，静默丢弃后续 A→B 报文的 UDP 中继
/// （QUIC 跑在 UDP 上，这是 `aa4c-transfer/tests/transfer.rs::cutting_proxy` 的 UDP
/// 版本：模拟真实网络分区/掉线，而非任何一方主动发 Cancel）。B→A 方向照常转发。
async fn cutting_udp_proxy(target: std::net::SocketAddr, cut_after: u64) -> std::net::SocketAddr {
    use tokio::net::UdpSocket;

    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65_535];
        let mut client_addr: Option<std::net::SocketAddr> = None;
        let mut forwarded = 0u64;
        let mut cut = false;
        loop {
            let (n, from) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(_) => break,
            };
            if from == target {
                // B → A：一路转发，不掐断（连接的死活只由 A→B 方向的黑洞决定）
                if let Some(c) = client_addr {
                    let _ = socket.send_to(&buf[..n], c).await;
                }
                continue;
            }
            // A → B
            client_addr = Some(from);
            if cut {
                continue; // 黑洞：静默丢弃，不回任何错误（模拟真实网络分区）
            }
            let _ = socket.send_to(&buf[..n], target).await;
            forwarded += n as u64;
            if forwarded >= cut_after {
                cut = true;
            }
        }
    });
    addr
}

#[tokio::test]
async fn quic_resume_after_disconnect() {
    // A 出站强制走 QUIC。QUIC 自带 keep-alive + 8s 空闲超时（见 aa4c-transfer::quic::
    // transport_config）：连接真正存活时 keep-alive 持续续命，只有黑洞代理让 keep-alive
    // 也送不出去的真断连才会在约 8s 内被两端各自发现——这就是本测试的计时来源。
    let a = spawn_node_quic().await;
    let b = spawn_node().await;
    let b_id = b.core.self_info().id;

    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    // 让 A 解析 B 时走 UDP 黑洞代理（而不是 B 的真实端口）：6 MiB 后黑洞，
    // 确保第一块（4 MiB）完整落盘、第二块写到一半时"断网"。
    let proxy = cutting_udp_proxy(peer_info(&b).addr.unwrap(), 6 * 1024 * 1024).await;
    let mut rec = a.core.store.get_device(&b_id).await.unwrap().unwrap();
    rec.last_addr = Some(proxy.to_string());
    a.core.store.upsert_device(&rec).await.unwrap();

    let recv_dir = b._dir.path().join("inbox");
    let ev_b2 = b.core.subscribe();
    let b_core = b.core.clone();
    let recv_dir2 = recv_dir.clone();
    tokio::spawn(async move {
        let mut rx = ev_b2;
        while let Ok(event) = rx.recv().await {
            if let CoreEvent::TransferRequest { task } = event {
                b_core
                    .accept_transfer(&task.id, true, Some(recv_dir2.clone()))
                    .await
                    .unwrap();
            }
        }
    });

    let src = a._dir.path().join("big.bin");
    let content: Vec<u8> = (0..20 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    tokio::fs::write(&src, &content).await.unwrap();

    // —— 第一次尝试：经黑洞代理，中途"断网" ——
    let ev_a1 = a.core.subscribe();
    let task1 = a.core.send_files(&b_id, vec![src.clone()]).await.unwrap();
    assert!(
        !timeout(Duration::from_secs(30), wait_terminal(ev_a1, &task1))
            .await
            .expect("first attempt reaches terminal state within 30s"),
        "first attempt must fail (blackholed)"
    );

    let part = recv_dir.join("big.bin.aa4c-part");
    let partial_len = tokio::fs::metadata(&part)
        .await
        .unwrap_or_else(|e| {
            panic!("expected partial file kept for resume (not explicit cancel): {e}")
        })
        .len();
    assert!(
        partial_len >= 4 * 1024 * 1024,
        "at least one full chunk should have landed before the blackhole, got {partial_len}"
    );
    assert!(partial_len < content.len() as u64);

    // —— 第二次尝试：把 last_addr 改回 B 的真实地址，重新发起，应从续传起点继续 ——
    let mut rec = a.core.store.get_device(&b_id).await.unwrap().unwrap();
    rec.last_addr = peer_info(&b).addr.map(|a| a.to_string());
    a.core.store.upsert_device(&rec).await.unwrap();

    let ev_a2 = a.core.subscribe();
    let task2 = a.core.send_files(&b_id, vec![src]).await.unwrap();
    assert!(
        timeout(WAIT, wait_terminal(ev_a2, &task2)).await.unwrap(),
        "second attempt (resumed) should succeed"
    );

    // 内容完整正确（证明「重新流式读前缀喂 hasher」+「从续传偏移继续」的整套逻辑没错）
    assert_eq!(
        tokio::fs::read(recv_dir.join("big.bin")).await.unwrap(),
        content
    );
    assert!(!part.exists(), "part file renamed away after success");

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_marks_stale_tasks_failed() {
    let dir = tempfile::tempdir().unwrap();
    // 第一次启动：手工插入一个「传输中」任务，模拟上次异常退出
    {
        let mut config = CoreConfig::new(dir.path().to_path_buf());
        config.listen_port = 0;
        config.transfer.default_save_dir = dir.path().join("downloads");
        let core = Core::start(config).await.unwrap();
        // 任务的 peer 需先存在于 devices 表（外键约束）
        core.store
            .upsert_device(&aa4c_store::DeviceRecord {
                id: "peer".into(),
                name: "Peer".into(),
                platform: aa4c_types::Platform::Macos,
                public_key: vec![0u8; 32],
                trusted: true,
                trust_level: aa4c_types::TrustLevel::Friend,
                paired_at: Some(1),
                last_seen_at: Some(1),
                last_addr: None,
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
        core.store
            .insert_task(&aa4c_types::TransferTask {
                id: "stale-1".into(),
                direction: aa4c_types::Direction::Recv,
                peer: "peer".into(),
                files: vec![],
                status: TransferStatus::Transferring,
                total_bytes: 0,
                transferred_bytes: 0,
                created_at: 1,
                error: None,
            })
            .await
            .unwrap();
        core.shutdown().await.unwrap();
    }
    // 第二次启动：遗留任务应被标记为失败
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    let core = Core::start(config).await.unwrap();
    let tasks = core.list_transfers(10, 0).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TransferStatus::Failed);
    core.shutdown().await.unwrap();
}
