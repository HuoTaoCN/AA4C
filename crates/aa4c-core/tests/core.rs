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
        .fetch_index(&b.core.self_info().id, Some(b_addr))
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
        .fetch_index(&b.core.self_info().id, Some(b_addr))
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
                server_hint: None,
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

// —— V0.3 里程碑 C2：aa4c-server 信令面 + 客户端接入 ——
//
// 内嵌启动一个真实 aa4c-server（同进程后台任务，ephemeral 端口），驱动真实 Core 通过它
// 完成注册/查询。`register_once`/`lookup_once` 是 aa4c-core 的 crate 内部函数，这里只能
// （也应该只）通过公开的 Core 方法间接验证——协议层本身的挑战/名单校验已由
// aa4c-server 自己的单元测试覆盖（见 crates/aa4c-server/src/lib.rs）。

/// 启动一个内嵌 aa4c-server，返回其句柄（身份数据目录需存活至测试结束，一并返回）
/// 与 `aa4c://127.0.0.1:port#fp` 地址。
async fn spawn_server() -> (Arc<aa4c_server::Server>, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let server = aa4c_server::run(aa4c_server::ServerConfig {
        data_dir: dir.path().to_path_buf(),
        listen_addr: "127.0.0.1:0".parse().unwrap(),
    })
    .await
    .expect("server starts");
    let url = server.address_with_host("127.0.0.1");
    (server, dir, url)
}

async fn enable_remote(core: &Core, server_url: &str) {
    let mut settings = core.get_settings().await.unwrap();
    settings.enable_remote = true;
    settings.server_url = Some(server_url.to_string());
    core.update_settings(settings).await.unwrap();
}

/// 轮询直到操作成功或耗尽重试次数：服务器注册/续约是后台异步任务，没有可等待的事件
/// （C4 才会补连接质量事件），测试里用短轮询代替固定 sleep，减少不必要的等待。
async fn retry_send_files(
    core: &Core,
    peer: &aa4c_types::DeviceId,
    paths: Vec<PathBuf>,
) -> aa4c_types::Result<String> {
    let mut last_err = None;
    for _ in 0..50 {
        match core.send_files(peer, paths.clone()).await {
            Ok(id) => return Ok(id),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_err.expect("at least one attempt"))
}

/// 等文件送达对端并落盘（复用 two_cores_pair_then_transfer 的等待模式）。
async fn wait_transfer_done(mut events: broadcast::Receiver<CoreEvent>, task_id: &str) {
    let ok = timeout(WAIT, async {
        loop {
            match events.recv().await.unwrap() {
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
    assert!(ok);
}

/// 全链路集成测试：resolve_peer 在没有 mDNS/落库地址时能靠服务器 Lookup 兜底找到对端
/// 并完成真实传输；解除配对后，即便还能解析出一个地址，B 本地也已经不认这份证书，
/// 传输必须以失败收场。
///
/// ⚠️ 这台机器上真实 mDNS 组播确实会在几百毫秒内找到本机的另一个 Core 实例（不像
/// GitHub Actions CI 那样天然无组播，见其余 e2e 测试"不依赖 mDNS"的假设）——所以第一段
/// 的"成功"不能严格证明连接**只**靠 Lookup 走通（可能 mDNS 也顺带命中了）。Lookup 协议
/// 本身（含允许名单校验、吊销语义）的确定性证明在 `server_link.rs` 的单测里（不经过
/// mDNS/Core，直接调用 `register_once`/`lookup_once`）；这里验证的是更根本的性质：
/// 无论地址怎么解析到的，B 端的信任判定才是真正的安全边界，解除配对后必然拒绝。
#[tokio::test]
async fn resolve_peer_reaches_peer_remotely_and_unpair_still_blocks_transfer() {
    let a = spawn_node().await;
    let b = spawn_node().await;
    let a_id = a.core.self_info().id;
    let b_id = b.core.self_info().id;
    let (_server, _server_dir, server_url) = spawn_server().await;

    // —— 配对（一如既往走本地地址，V0.3 不改配对本身）——
    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    // 两端都开启远程、指向同一个内嵌服务器（CONNECT_DESIGN §1.1：自己的多台设备
    // 共用同一服务器，是本里程碑 resolve_peer 兜底覆盖的主场景）
    enable_remote(&a.core, &server_url).await;
    enable_remote(&b.core, &server_url).await;

    // 抹掉 A 本地记的 B 地址：逼 resolve_peer 走到落库地址之后的兜底路径
    // （mDNS 快照仍可能命中，见函数级注释）。
    let mut rec = a.core.store.get_device(&b_id).await.unwrap().unwrap();
    rec.last_addr = None;
    a.core.store.upsert_device(&rec).await.unwrap();

    // —— 建连传输：等 A/B 的后台注册续约生效后重试 ——
    let recv_dir = b._dir.path().join("inbox");
    let src = a._dir.path().join("hello.txt");
    tokio::fs::write(&src, b"via server lookup").await.unwrap();
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
    let ev_a2 = a.core.subscribe();
    let task_id = retry_send_files(&a.core, &b_id, vec![src.clone()])
        .await
        .expect("send succeeds once address resolution catches up");
    wait_transfer_done(ev_a2, &task_id).await;
    assert_eq!(
        tokio::fs::read(recv_dir.join("hello.txt")).await.unwrap(),
        b"via server lookup"
    );

    // —— 解除配对：B 本地不再认 A 的证书，之后任何一次传输尝试都必须失败 ——
    // send_files 的 Ok(task_id) 只代表"解析到了一个地址、任务已排队"（可能是 mDNS 快照，
    // 也可能是服务器上尚未过期的旧登记），不代表对端接受；真正的判定要等异步事件。
    b.core.unpair_device(&a_id).await.unwrap();
    let src2 = a._dir.path().join("after-unpair.txt");
    tokio::fs::write(&src2, b"should not reach").await.unwrap();
    let mut ev_a3 = a.core.subscribe();
    let task_id2 = retry_send_files(&a.core, &b_id, vec![src2.clone()])
        .await
        .expect("an address can still be resolved (mDNS and/or stale server registration)");
    let succeeded = timeout(WAIT, async {
        loop {
            match ev_a3.recv().await.unwrap() {
                CoreEvent::TransferFailed { task_id: t, .. } if t == task_id2 => return false,
                CoreEvent::TransferDone { task_id: t } if t == task_id2 => return true,
                _ => {}
            }
        }
    })
    .await
    .expect("reaches terminal state");
    assert!(
        !succeeded,
        "B revoked A; transfer must fail even though an address was resolved"
    );

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

#[tokio::test]
async fn server_lookup_denies_device_not_in_allow_list() {
    let b = spawn_node().await;
    let c = spawn_node().await; // 从未与 B 配对，冒充"以为自己配对了"
    let b_id = b.core.self_info().id;
    let (_server, _server_dir, server_url) = spawn_server().await;

    enable_remote(&b.core, &server_url).await;
    enable_remote(&c.core, &server_url).await;

    // C 在本地伪造一条"已配对"的 B 记录（真实攻击者能做到的事：篡改自己的本地状态）；
    // 服务器端的允许名单校验只认 B 自己上报的名单，不受 C 本地怎么想影响——这才是
    // 安全边界真正生效的地方。
    c.core
        .store
        .upsert_device(&aa4c_store::DeviceRecord {
            id: b_id.clone(),
            name: "冒充的B".into(),
            platform: aa4c_types::Platform::Macos,
            public_key: vec![9u8; 32],
            trusted: true,
            trust_level: aa4c_types::TrustLevel::Friend,
            paired_at: Some(1),
            last_seen_at: Some(1),
            last_addr: None,
            server_hint: None,
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();

    // B 已在 enable_remote() 里触发过一次立即注册（允许名单只含它真正配对过的设备，
    // 不含 C）；下面的重试循环本身就会给这次异步注册留够落地时间，不需要额外等待。
    //
    // send_files 的 Ok(task_id) 只代表"解析到了一个地址"（C 本地伪造的记录、mDNS 快照都
    // 可能提供地址），不代表 B 会接受——B 服务端看到的是 C 的真实证书指纹，从未配对过，
    // 真正的判定要等异步事件：不管地址怎么来的，B 本地的信任表才是安全边界。
    let src = c._dir.path().join("nope.txt");
    tokio::fs::write(&src, b"should not reach").await.unwrap();
    let ev_c = c.core.subscribe();
    let task_id = retry_send_files(&c.core, &b_id, vec![src.clone()])
        .await
        .expect("an address can still be resolved (mDNS and/or C's fabricated local record)");
    let succeeded = timeout(WAIT, async {
        let mut rx = ev_c;
        loop {
            match rx.recv().await.unwrap() {
                CoreEvent::TransferFailed { task_id: t, .. } if t == task_id => return false,
                CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
                _ => {}
            }
        }
    })
    .await
    .expect("reaches terminal state");
    assert!(
        !succeeded,
        "C is not in B's allow list / was never paired; transfer must fail"
    );

    b.core.shutdown().await.unwrap();
    c.core.shutdown().await.unwrap();
}

/// 里程碑 C3 验收：强制走连接阶梯第 4 档（中继）完成一次真实文件传输
/// （V0.3_IMPLEMENTATION_PLAN.md C3「e2e：强制走中继路径完成一次传输」）。
///
/// 局域网直连 / 公网直连都要让它们确定性地失败，而不是依赖真实网络不可达（这台开发机
/// 上一切本来就互通）：关掉 A 自己的 mDNS 浏览（`resolve_peer` 的第一档直接找不到任何
/// 设备），把 A 落库的 B 最后地址钉成本机一个确定没有监听者的端口（第二档直连立即被
/// connection refused，而不是等超时）。剩下唯一能用的就是中继——服务器把 A、B 的连接
/// 撮合起来，设备间 mTLS 在这条裸管道上原样握手，ATP 原样跑完。
#[tokio::test]
async fn forced_relay_path_completes_a_transfer() {
    let a = spawn_node().await;
    let b = spawn_node().await;
    let b_id = b.core.self_info().id;
    let (_server, _server_dir, server_url) = spawn_server().await;

    // 正常配对（局域网直连，配对本身不在本里程碑范围内）
    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    enable_remote(&a.core, &server_url).await;
    enable_remote(&b.core, &server_url).await;
    // B 的常驻连接（`server_link::spawn_register_loop`）需要真实的连接+握手时间才能把自己
    // 登记成"可被推送 IncomingRelay"（`register_notify` 只消掉轮询等待，消不掉这段真实
    // 网络往返）；没有可等待的事件（C4 才会补连接质量事件），这里用短暂定长等待让它
    // 落地，避免第一次中继请求因为 B 还没注册上而白白等满一轮 token TTL 才失败重试。
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 逼连接阶梯只剩中继这一档：关掉 A 自己的 mDNS（第 1 档），把 B 的落库地址钉成
    // 一个确定没人监听的本机端口（第 2 档立即 connection refused，不必等超时）。
    a.core.discovery.stop().await.unwrap();
    let mut rec = a.core.store.get_device(&b_id).await.unwrap().unwrap();
    rec.last_addr = Some("127.0.0.1:1".to_string());
    a.core.store.upsert_device(&rec).await.unwrap();

    let recv_dir = b._dir.path().join("inbox");
    let src = a._dir.path().join("via-relay.txt");
    tokio::fs::write(&src, b"delivered purely through relay")
        .await
        .unwrap();
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

    let ev_a2 = a.core.subscribe();
    let task_id = retry_send_files(&a.core, &b_id, vec![src.clone()])
        .await
        .expect("relay dialer is wired, send queues even with no direct address");
    wait_transfer_done(ev_a2, &task_id).await;
    assert_eq!(
        tokio::fs::read(recv_dir.join("via-relay.txt"))
            .await
            .unwrap(),
        b"delivered purely through relay"
    );

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}

/// 里程碑 C4 验收：跨设备索引交换接入完整连接阶梯——完全信任设备即使不在同一局域网
/// （也够不到落库地址），也能通过自建服务器 + 中继同步到对方的共享索引
/// （CONNECT_DESIGN.md §6「远程同步」；此前 `sync_exchange` 只认 mDNS 在线快照，是本
/// 里程碑要补的缺口）。复用 `forced_relay_path_completes_a_transfer` 的同一套「逼连接
/// 阶梯只剩中继」手法。
#[tokio::test]
async fn remote_index_exchange_reaches_peer_via_relay() {
    let a = spawn_node().await;
    let b = spawn_node().await;
    let b_id = b.core.self_info().id;
    let (_server, _server_dir, server_url) = spawn_server().await;

    let ev_a = a.core.subscribe();
    let ev_b = b.core.subscribe();
    a.core.pairing.start_pairing(&peer_info(&b)).await.unwrap();
    let (ok_a, ok_b) = tokio::join!(
        timeout(WAIT, drive_pairing(a.core.clone(), ev_a)),
        timeout(WAIT, drive_pairing(b.core.clone(), ev_b)),
    );
    assert!(ok_a.unwrap() && ok_b.unwrap(), "both sides pair");

    enable_remote(&a.core, &server_url).await;
    enable_remote(&b.core, &server_url).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // 让 B 的常驻连接落地，见另一个测试的注释

    // 逼连接阶梯只剩中继这一档（同 forced_relay_path_completes_a_transfer）
    a.core.discovery.stop().await.unwrap();
    let mut rec = a.core.store.get_device(&b_id).await.unwrap().unwrap();
    rec.last_addr = Some("127.0.0.1:1".to_string());
    a.core.store.upsert_device(&rec).await.unwrap();

    // B 建一个共享文件夹
    let shared = b._dir.path().join("shared");
    tokio::fs::create_dir_all(&shared).await.unwrap();
    tokio::fs::write(shared.join("doc.txt"), b"via relay index exchange")
        .await
        .unwrap();
    b.core.add_sync_scope(shared.clone()).await.unwrap();
    b.core.rescan_sync().await.unwrap();

    // B 也把 A 标为「我的设备」，否则会拒绝交出索引（同 index_exchange_gated_by_full_trust）
    let a_id = a.core.self_info().id;
    b.core
        .set_trust_level(&a_id, aa4c_types::TrustLevel::Full)
        .await
        .unwrap();

    // A 把 B 标为「我的设备」：这一步内部会立即尝试拉一次索引（orchestrate::set_trust_level），
    // 此前的实现只在 B 处于 A 的 mDNS 在线快照里才会真的发起——这里 A 自己的 mDNS 已关闭，
    // 必须靠连接阶梯（落库地址已被钉成死地址 → 中继）才能够到 B。
    a.core
        .set_trust_level(&b_id, aa4c_types::TrustLevel::Full)
        .await
        .unwrap();

    let remote = a.core.store.list_remote_index().await.unwrap();
    assert_eq!(remote.len(), 1, "remote index synced purely through relay");
    assert_eq!(remote[0].rel_path, "shared/doc.txt");
    assert_eq!(remote[0].device_id, b_id);

    a.core.shutdown().await.unwrap();
    b.core.shutdown().await.unwrap();
}
