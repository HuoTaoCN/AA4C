//! 传输引擎端到端测试（V0.1_IMPLEMENTATION_PLAN.md M5）。
//!
//! 双实例（同进程、loopback TLS）：单文件 / 空文件 / 深层目录 / 大文件 /
//! 拒绝 / 取消 / 断连 / 未配对拒绝。1GB 大文件标 #[ignore] 本地运行。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aa4c_identity::Identity;
use aa4c_store::{DeviceRecord, Store};
use aa4c_transfer::{TransferConfig, TransferService};
use aa4c_types::{CoreEvent, DeviceInfo, Platform, TransferStatus, TrustLevel};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);

struct Node {
    identity: Arc<Identity>,
    store: Store,
    svc: Arc<TransferService>,
    events: broadcast::Sender<CoreEvent>,
    device: DeviceInfo,
    save_dir: PathBuf,
    _dir: tempfile::TempDir,
}

async fn spawn_node(name: &str, max_concurrent_tasks: usize) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let identity = Arc::new(Identity::load_or_generate(dir.path()).unwrap());
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
    let (tx, _) = broadcast::channel(256);
    let save_dir = dir.path().join("recv");

    let svc = TransferService::new(
        identity.clone(),
        store.clone(),
        tx.clone(),
        TransferConfig {
            default_save_dir: save_dir.clone(),
            timeout: Duration::from_secs(8),
            max_concurrent_tasks,
            ..TransferConfig::default()
        },
    );
    let port = svc.start_listener(0).await.unwrap();
    let device = DeviceInfo {
        id: identity.device_id().clone(),
        name: name.into(),
        platform: Platform::Macos,
        version: "0.1.0".into(),
        addr: Some(format!("127.0.0.1:{port}").parse().unwrap()),
        online: true,
        trusted: true,
        trust_level: Some(TrustLevel::Friend),
    };
    Node {
        identity,
        store,
        svc,
        events: tx,
        device,
        save_dir,
        _dir: dir,
    }
}

/// 模拟已完成配对：把对方写入本地 devices 表（trusted=1）。
async fn trust(node: &Node, peer: &Node) {
    node.store
        .upsert_device(&DeviceRecord {
            id: peer.device.id.clone(),
            name: peer.device.name.clone(),
            platform: peer.device.platform,
            public_key: peer.identity.public_key().to_vec(),
            trusted: true,
            trust_level: TrustLevel::Friend,
            paired_at: Some(1),
            last_seen_at: Some(1),
            last_addr: peer.device.addr.map(|a| a.to_string()),
            server_hint: None,
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
}

async fn pair_nodes(a: &Node, b: &Node) {
    trust(a, b).await;
    trust(b, a).await;
}

/// 接收端自动接受驱动。
fn auto_accept(node: &Node) {
    let mut rx = node.events.subscribe();
    let svc = node.svc.clone();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let CoreEvent::TransferRequest { task } = event {
                let _ = svc.accept(&task.id, true, None).await;
            }
        }
    });
}

/// 等待指定任务的终态事件，返回是否成功。
/// 注意：`rx` 必须在 `send()` 之前订阅，避免错过小文件的瞬时完成事件。
async fn wait_terminal(mut rx: broadcast::Receiver<CoreEvent>, task_id: &str) -> bool {
    timeout(WAIT, async {
        loop {
            match rx.recv().await.expect("event bus open") {
                CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
                CoreEvent::TransferFailed { task_id: t, .. } if t == task_id => return false,
                _ => {}
            }
        }
    })
    .await
    .expect("task should reach terminal state in time")
}

/// 轮询 store 直到任务终态（与事件无关，无竞态）。
async fn task_status(store: &Store, task_id: &str) -> TransferStatus {
    for _ in 0..200 {
        if let Some(task) = store
            .list_tasks(20, 0)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.id == task_id)
        {
            if task.status.is_terminal() {
                return task.status;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {task_id} never reached terminal state");
}

fn write_patterned(path: &Path, size: usize) {
    let mut data = vec![0u8; size];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    std::fs::write(path, data).unwrap();
}

fn hash_file(path: &Path) -> String {
    blake3::hash(&std::fs::read(path).unwrap())
        .to_hex()
        .to_string()
}

fn assert_no_part_files(dir: &Path) {
    if !dir.exists() {
        return;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else {
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(!name.ends_with(".aa4c-part"), "leftover part file: {name}");
            }
        }
    }
}

#[tokio::test]
async fn single_file_roundtrip_with_hash_and_store() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src = a._dir.path().join("IMG 测试.jpg");
    write_patterned(&src, 3 * 1024 * 1024 + 17);

    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src.clone()]).await.unwrap();
    assert!(wait_terminal(rx, &task_id).await, "send should succeed");

    let dst = b.save_dir.join("IMG 测试.jpg");
    assert_eq!(hash_file(&src), hash_file(&dst));
    assert_eq!(task_status(&a.store, &task_id).await, TransferStatus::Done);
    assert_eq!(task_status(&b.store, &task_id).await, TransferStatus::Done);
    assert_no_part_files(&b.save_dir);
}

#[tokio::test]
async fn empty_file_transfers() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src = a._dir.path().join("empty.txt");
    std::fs::write(&src, b"").unwrap();
    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src]).await.unwrap();
    assert!(wait_terminal(rx, &task_id).await);
    let dst = b.save_dir.join("empty.txt");
    assert_eq!(std::fs::metadata(&dst).unwrap().len(), 0);
}

#[tokio::test]
async fn folder_structure_is_preserved_and_duplicates_renamed() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let root = a._dir.path().join("项目 X");
    std::fs::create_dir_all(root.join("src/深层 目录")).unwrap();
    write_patterned(&root.join("readme.md"), 100);
    write_patterned(&root.join("src/深层 目录/数据.bin"), 5000);

    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![root.clone()]).await.unwrap();
    assert!(wait_terminal(rx, &task_id).await);
    assert_eq!(
        hash_file(&root.join("src/深层 目录/数据.bin")),
        hash_file(&b.save_dir.join("项目 X/src/深层 目录/数据.bin")),
    );

    // 再发一次：重名自动加 (1)
    let rx2 = a.events.subscribe();
    let task2 = a.svc.send(&b.device, vec![root.clone()]).await.unwrap();
    assert!(wait_terminal(rx2, &task2).await);
    assert!(b.save_dir.join("项目 X/readme (1).md").exists());
}

#[tokio::test]
async fn medium_file_hash_matches() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src = a._dir.path().join("big.bin");
    write_patterned(&src, 32 * 1024 * 1024 + 333); // 跨多个 4MiB 分块且非整块
    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src.clone()]).await.unwrap();
    assert!(wait_terminal(rx, &task_id).await);
    assert_eq!(hash_file(&src), hash_file(&b.save_dir.join("big.bin")));
}

#[tokio::test]
#[ignore = "1GB 大文件，本地手动运行：cargo test -p aa4c-transfer -- --ignored"]
async fn gigabyte_file_hash_matches() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src = a._dir.path().join("1g.bin");
    write_patterned(&src, 1024 * 1024 * 1024);
    let mut rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src.clone()]).await.unwrap();
    let ok = timeout(Duration::from_secs(120), async {
        loop {
            match rx.recv().await.unwrap() {
                CoreEvent::TransferDone { task_id: t } if t == task_id => return true,
                CoreEvent::TransferFailed { task_id: t, .. } if t == task_id => return false,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(ok);
    assert_eq!(hash_file(&src), hash_file(&b.save_dir.join("1g.bin")));
}

#[tokio::test]
async fn receiver_rejecting_marks_both_sides() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;

    // 手动拒绝
    let mut rx = b.events.subscribe();
    let svc_b = b.svc.clone();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let CoreEvent::TransferRequest { task } = event {
                let _ = svc_b.accept(&task.id, false, None).await;
            }
        }
    });

    let src = a._dir.path().join("f.txt");
    write_patterned(&src, 10);
    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src]).await.unwrap();
    assert!(!wait_terminal(rx, &task_id).await, "must fail");
    assert_eq!(
        task_status(&a.store, &task_id).await,
        TransferStatus::Rejected
    );
    assert_eq!(
        task_status(&b.store, &task_id).await,
        TransferStatus::Rejected
    );
}

#[tokio::test]
async fn sender_cancel_mid_transfer_cleans_up() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src = a._dir.path().join("huge.bin");
    write_patterned(&src, 256 * 1024 * 1024);

    let task_id = a.svc.send(&b.device, vec![src]).await.unwrap();

    // 第一个进度事件后立即取消（仍在传输中）
    let mut rx = a.events.subscribe();
    timeout(WAIT, async {
        loop {
            if let Ok(CoreEvent::TransferProgress { task_id: t, .. }) = rx.recv().await {
                if t == task_id {
                    return;
                }
            }
        }
    })
    .await
    .expect("progress event expected");
    a.svc.cancel(&task_id).await.unwrap();

    assert_eq!(
        task_status(&a.store, &task_id).await,
        TransferStatus::Cancelled
    );
    let b_status = task_status(&b.store, &task_id).await;
    assert!(
        matches!(b_status, TransferStatus::Failed | TransferStatus::Cancelled),
        "receiver should not be Done, got {b_status:?}"
    );
    assert_no_part_files(&b.save_dir);
}

/// 在 N 字节后切断连接的 TCP 代理（模拟传输中断连/杀进程）。
async fn cutting_proxy(target: SocketAddr, cut_after: u64) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut client, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut server) = tokio::net::TcpStream::connect(target).await else {
            return;
        };
        let (mut cr, mut cw) = client.split();
        let (mut sr, mut sw) = server.split();
        let upstream = async {
            let mut sent = 0u64;
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match cr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if sw.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                sent += n as u64;
                if sent >= cut_after {
                    break; // 掐断
                }
            }
        };
        let downstream = async {
            let _ = tokio::io::copy(&mut sr, &mut cw).await;
        };
        tokio::select! {
            () = upstream => {}
            () = downstream => {}
        }
        // 任一方向结束即丢弃两端连接
    });
    addr
}

#[tokio::test]
async fn mid_transfer_disconnect_fails_both_sides() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    // 经代理发送，6 MiB 后断开：默认 chunk_size=4 MiB，6 MiB 确保第一块完整落盘、
    // 第二块写到一半时断连，才有「完整前缀 + 不完整尾部」可断言（而非卡在第一块中途）。
    let proxy = cutting_proxy(b.device.addr.unwrap(), 6 * 1024 * 1024).await;
    let mut peer = b.device.clone();
    peer.addr = Some(proxy);

    let src = a._dir.path().join("doomed.bin");
    write_patterned(&src, 64 * 1024 * 1024);
    let task_id = a.svc.send(&peer, vec![src]).await.unwrap();

    assert_eq!(
        task_status(&a.store, &task_id).await,
        TransferStatus::Failed
    );
    assert_eq!(
        task_status(&b.store, &task_id).await,
        TransferStatus::Failed
    );
    // V0.3 里程碑 C1（断点续传）起，非「明确取消」的中断（这里是网络断连）**保留**
    // .aa4c-part 文件，供下次重新发起时续传（PROTOCOL.md §13）；只有明确取消
    // （本地用户取消 / 对端主动 Cancel）才清理，见 sender_cancel_mid_transfer_cleans_up。
    let part = b.save_dir.join("doomed.bin.aa4c-part");
    let len = std::fs::metadata(&part)
        .unwrap_or_else(|e| panic!("expected partial file kept for resume: {e}"))
        .len();
    assert!(
        len >= 4 * 1024 * 1024,
        "at least one full 4 MiB chunk should have landed before the cut, got {len}"
    );
    assert!(len < 64 * 1024 * 1024, "partial file should be incomplete");
}

#[tokio::test]
async fn unpaired_sender_is_refused() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    // 只有 A 信任 B；B 不认识 A（未配对）
    trust(&a, &b).await;

    let src = a._dir.path().join("f.txt");
    write_patterned(&src, 10);
    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![src]).await.unwrap();
    assert!(!wait_terminal(rx, &task_id).await, "must be refused");
    assert_eq!(
        task_status(&a.store, &task_id).await,
        TransferStatus::Failed
    );
    // 接收端没有任何落盘
    assert_no_part_files(&b.save_dir);
    assert!(b.store.list_tasks(10, 0).await.unwrap().is_empty());
}

/// `TransferConfig::max_concurrent_tasks` 真的限流，不是摆设：`send()` 立刻
/// 返回 task_id（DB 行、事件订阅都马上就绪），真正的网络工作在后台任务里排队
/// 等 `send_permits` 信号量——这条测试验证的正是那个信号量，不是"两个任务
/// 最终都成功"（这点其他测试已经覆盖）。
///
/// 用事件顺序断言而不是掐时间点：不管两个任务谁先抢到许可证（不保证 FIFO），
/// 只要「后拿到许可证开始连接的那个」的 `TransferConnected` 严格晚于「先开始
/// 的那个」的终态事件，就证明同一时刻只有一个任务在真正跑传输。
#[tokio::test]
async fn max_concurrent_tasks_serializes_transfers() {
    let a = spawn_node("发送方", 1).await;
    let b = spawn_node("接收方", 1).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    let src1 = a._dir.path().join("one.bin");
    let src2 = a._dir.path().join("two.bin");
    write_patterned(&src1, 2 * 1024 * 1024);
    write_patterned(&src2, 2 * 1024 * 1024);

    let mut rx = a.events.subscribe();
    let task1 = a.svc.send(&b.device, vec![src1]).await.unwrap();
    let task2 = a.svc.send(&b.device, vec![src2]).await.unwrap();

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Kind {
        Connected,
        Terminal,
    }
    let mut seen: Vec<(String, Kind)> = Vec::new();
    let mut terminal_count = 0;
    timeout(WAIT, async {
        while terminal_count < 2 {
            match rx.recv().await.expect("event bus open") {
                CoreEvent::TransferConnected { task_id, .. }
                    if task_id == task1 || task_id == task2 =>
                {
                    seen.push((task_id, Kind::Connected));
                }
                CoreEvent::TransferDone { task_id } if task_id == task1 || task_id == task2 => {
                    seen.push((task_id, Kind::Terminal));
                    terminal_count += 1;
                }
                CoreEvent::TransferFailed { task_id, .. }
                    if task_id == task1 || task_id == task2 =>
                {
                    seen.push((task_id, Kind::Terminal));
                    terminal_count += 1;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("both tasks should reach a terminal state in time");

    let connected_idx = |id: &str| {
        seen.iter()
            .position(|(t, k)| t == id && *k == Kind::Connected)
    };
    let terminal_idx = |id: &str| {
        seen.iter()
            .position(|(t, k)| t == id && *k == Kind::Terminal)
    };
    let (first, second) = if connected_idx(&task1) < connected_idx(&task2) {
        (task1.clone(), task2.clone())
    } else {
        (task2.clone(), task1.clone())
    };
    assert!(
        connected_idx(&second).unwrap() > terminal_idx(&first).unwrap(),
        "second task started connecting before the first task reached a terminal state \
         — max_concurrent_tasks=1 did not serialize the two sends: {seen:?}"
    );

    assert_eq!(task_status(&a.store, &task1).await, TransferStatus::Done);
    assert_eq!(task_status(&a.store, &task2).await, TransferStatus::Done);
}

/// 大批量小文件：单文件/双文件场景之外，验证 `transfer_files` 逐文件循环在
/// 几十个文件规模下依然逐个正确落盘、无漏发无串号（`file_index` 靠数组下标
/// 隐式对应，文件一多最容易在这类"差一"错误上翻车）。
#[tokio::test]
async fn many_small_files_all_land_correctly() {
    let a = spawn_node("发送方", 4).await;
    let b = spawn_node("接收方", 4).await;
    pair_nodes(&a, &b).await;
    auto_accept(&b);

    const COUNT: usize = 50;
    let root = a._dir.path().join("批量");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..COUNT {
        write_patterned(&root.join(format!("file_{i:03}.bin")), 37 + i);
    }

    let rx = a.events.subscribe();
    let task_id = a.svc.send(&b.device, vec![root.clone()]).await.unwrap();
    assert!(wait_terminal(rx, &task_id).await);

    for i in 0..COUNT {
        let name = format!("file_{i:03}.bin");
        assert_eq!(
            hash_file(&root.join(&name)),
            hash_file(&b.save_dir.join("批量").join(&name)),
            "content mismatch for {name}"
        );
    }
    assert_no_part_files(&b.save_dir);
    assert_eq!(task_status(&a.store, &task_id).await, TransferStatus::Done);
}
