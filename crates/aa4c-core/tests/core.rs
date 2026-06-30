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
