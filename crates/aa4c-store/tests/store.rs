//! aa4c-store 集成测试（V0.1_IMPLEMENTATION_PLAN.md M1）。

use aa4c_store::{DeviceRecord, Store};
use aa4c_types::{
    Direction, FileStatus, Platform, RemoteIndexEntry, ScopeKind, SyncFileEntry, TransferFile,
    TransferStatus, TransferTask, TrustLevel,
};

fn sample_device(id: &str, trusted: bool) -> DeviceRecord {
    DeviceRecord {
        id: id.repeat(64 / id.len().max(1)),
        name: format!("设备-{id}"),
        platform: Platform::Macos,
        public_key: vec![7u8; 32],
        trusted,
        trust_level: TrustLevel::Friend,
        paired_at: trusted.then_some(1_750_000_000_000),
        last_seen_at: Some(1_750_000_000_000),
        last_addr: Some("192.168.1.10:42420".into()),
        server_hint: None,
        created_at: 0, // 由 Store 维护
        updated_at: 0,
    }
}

fn sample_task(id: &str, peer: &str, created_at: i64) -> TransferTask {
    TransferTask {
        id: id.into(),
        direction: Direction::Send,
        peer: peer.into(),
        files: vec![
            TransferFile {
                rel_path: "照片/IMG_0001.jpg".into(),
                size: 1024,
                hash: None,
                status: FileStatus::Pending,
            },
            TransferFile {
                rel_path: "照片/视频 (1).mp4".into(),
                size: 4 * 1024 * 1024,
                hash: Some("ab".repeat(32)),
                status: FileStatus::Done,
            },
        ],
        status: TransferStatus::WaitingAccept,
        total_bytes: 1024 + 4 * 1024 * 1024,
        transferred_bytes: 0,
        created_at,
        error: None,
    }
}

#[tokio::test]
async fn migration_is_idempotent_across_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("aa4c.db");

    let store = Store::open(&db_path).await.unwrap();
    drop(store);
    let store = Store::open(&db_path).await.unwrap();
    drop(store);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 7); // 001_init + 002_trust + 003_sync + 004_remote_index + 005_conflicts + 006_server_hint + 007_shares
}

#[tokio::test]
async fn device_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let device = sample_device("a", true);
    store.upsert_device(&device).await.unwrap();

    let got = store.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(got.name, device.name);
    assert_eq!(got.platform, Platform::Macos);
    assert_eq!(got.public_key, device.public_key);
    assert!(got.trusted);
    assert_eq!(got.trust_level, TrustLevel::Friend); // 配对默认朋友
    assert!(got.created_at > 0);
    assert_eq!(got.created_at, got.updated_at);

    // 升级 / 降级信任分级
    store
        .set_trust_level(&device.id, TrustLevel::Full)
        .await
        .unwrap();
    let upgraded = store.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(upgraded.trust_level, TrustLevel::Full);
    assert!(store
        .set_trust_level(&"missing".repeat(8), TrustLevel::Full)
        .await
        .is_err());

    // 对端 home server 地址（CONNECT_DESIGN §3.4，里程碑 C2）
    assert!(upgraded.server_hint.is_none());
    store
        .set_server_hint(
            &device.id,
            Some("aa4c://example.com:42420#abcd1234abcd1234".into()),
        )
        .await
        .unwrap();
    let hinted = store.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(
        hinted.server_hint.as_deref(),
        Some("aa4c://example.com:42420#abcd1234abcd1234")
    );
    assert!(store
        .set_server_hint(&"missing".repeat(8), None)
        .await
        .is_err());

    // upsert 更新：name 变化，created_at 保持
    let mut renamed = device.clone();
    renamed.name = "改名后的设备".into();
    store.upsert_device(&renamed).await.unwrap();
    let got2 = store.get_device(&device.id).await.unwrap().unwrap();
    assert_eq!(got2.name, "改名后的设备");
    assert_eq!(got2.created_at, got.created_at);

    // 未配对设备不出现在 paired 列表
    store
        .upsert_device(&sample_device("b", false))
        .await
        .unwrap();
    let paired = store.list_paired_devices().await.unwrap();
    assert_eq!(paired.len(), 1);
    assert_eq!(paired[0].id, device.id);

    store.remove_device(&device.id).await.unwrap();
    assert!(store.get_device(&device.id).await.unwrap().is_none());
}

#[tokio::test]
async fn task_with_files_roundtrip_and_updates() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let device = sample_device("c", true);
    store.upsert_device(&device).await.unwrap();

    let older = sample_task("task-old", &device.id, 1_000);
    let newer = sample_task("task-new", &device.id, 2_000);
    store.insert_task(&older).await.unwrap();
    store.insert_task(&newer).await.unwrap();

    // 倒序分页 + 文件明细完整往返
    let tasks = store.list_tasks(10, 0).await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "task-new");
    assert_eq!(tasks[1], older);
    assert_eq!(tasks[0].files.len(), 2);
    assert_eq!(tasks[0].files[1].rel_path, "照片/视频 (1).mp4");

    let page2 = store.list_tasks(1, 1).await.unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].id, "task-old");

    // 状态与进度更新
    store
        .update_task_progress(&"task-new".into(), 512)
        .await
        .unwrap();
    store
        .update_task_status(&"task-new".into(), TransferStatus::Failed, Some("连接断开"))
        .await
        .unwrap();
    let updated = &store.list_tasks(1, 0).await.unwrap()[0];
    assert_eq!(updated.transferred_bytes, 512);
    assert_eq!(updated.status, TransferStatus::Failed);
    assert_eq!(updated.error.as_deref(), Some("连接断开"));

    // 更新不存在的任务报错
    assert!(store
        .update_task_status(&"missing".into(), TransferStatus::Done, None)
        .await
        .is_err());
}

#[tokio::test]
async fn removing_device_cascades_to_tasks_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("aa4c.db");
    let store = Store::open(&db_path).await.unwrap();

    let device = sample_device("d", true);
    store.upsert_device(&device).await.unwrap();
    store
        .insert_task(&sample_task("t1", &device.id, 1))
        .await
        .unwrap();

    store.remove_device(&device.id).await.unwrap();
    assert!(store.list_tasks(10, 0).await.unwrap().is_empty());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let files: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(files, 0);
}

#[tokio::test]
async fn settings_roundtrip_and_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    assert!(store.get_setting("device_name").await.unwrap().is_none());
    store
        .set_setting("device_name", "\"Huo 的 MacBook\"")
        .await
        .unwrap();
    store
        .set_setting("device_name", "\"客厅电脑\"")
        .await
        .unwrap();
    assert_eq!(
        store.get_setting("device_name").await.unwrap().as_deref(),
        Some("\"客厅电脑\"")
    );
}

#[tokio::test]
async fn sync_scope_index_diffing_and_cascade_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let scope = store
        .upsert_sync_scope(ScopeKind::Folder, "/Users/x/Photos")
        .await
        .unwrap();
    assert_eq!(scope.kind, ScopeKind::Folder);

    // 同路径再 upsert 一次：返回同一行，不重复创建
    let again = store
        .upsert_sync_scope(ScopeKind::Folder, "/Users/x/Photos")
        .await
        .unwrap();
    assert_eq!(again.id, scope.id);
    assert_eq!(store.list_sync_scopes().await.unwrap().len(), 1);

    let entry = |rel: &str, size: u64| SyncFileEntry {
        scope_id: scope.id.clone(),
        rel_path: rel.into(),
        size,
        mtime: 1000,
        hash: Some("h1".into()),
        present_local: true,
    };
    store
        .replace_scope_index(&scope.id, vec![entry("a.jpg", 1), entry("b.jpg", 2)])
        .await
        .unwrap();
    let mut idx = store.list_scope_index(&scope.id).await.unwrap();
    idx.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    assert_eq!(idx.len(), 2);
    assert_eq!(idx[0].rel_path, "a.jpg");

    // 第二次扫描：a.jpg 消失、b.jpg 改了 size、新增 c.jpg —— 一次性 diff 落库
    store
        .replace_scope_index(&scope.id, vec![entry("b.jpg", 99), entry("c.jpg", 3)])
        .await
        .unwrap();
    let mut idx2 = store.list_scope_index(&scope.id).await.unwrap();
    idx2.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let rels: Vec<_> = idx2.iter().map(|e| e.rel_path.as_str()).collect();
    assert_eq!(rels, vec!["b.jpg", "c.jpg"]);
    assert_eq!(idx2[0].size, 99);

    assert_eq!(store.list_all_sync_files().await.unwrap().len(), 2);

    // 删除范围级联清空其索引
    store.remove_sync_scope(&scope.id).await.unwrap();
    assert!(store.list_sync_scopes().await.unwrap().is_empty());
    assert!(store.list_all_sync_files().await.unwrap().is_empty());
}

#[tokio::test]
async fn ensure_inbox_scope_is_singleton_and_path_mutable() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let inbox = store
        .ensure_inbox_scope("/Users/x/Downloads/AA4C")
        .await
        .unwrap();
    assert_eq!(inbox.kind, ScopeKind::Inbox);

    // 路径不变：原地返回同一条
    let same = store
        .ensure_inbox_scope("/Users/x/Downloads/AA4C")
        .await
        .unwrap();
    assert_eq!(same.id, inbox.id);

    // save_dir 变更：同一条记录原地更新路径，不会新增一行
    let moved = store
        .ensure_inbox_scope("/Users/x/AA4C-Inbox")
        .await
        .unwrap();
    assert_eq!(moved.id, inbox.id);
    assert_eq!(moved.local_path, "/Users/x/AA4C-Inbox");

    let scopes = store.list_sync_scopes().await.unwrap();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].local_path, "/Users/x/AA4C-Inbox");
}

#[tokio::test]
async fn remote_index_replace_clear_and_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let device = sample_device("e", true);
    store.upsert_device(&device).await.unwrap();

    let entry = |rel: &str, size: u64| RemoteIndexEntry {
        device_id: device.id.clone(),
        rel_path: rel.into(),
        size,
        hash: Some("h".into()),
        seen_at: 1000,
    };
    store
        .replace_remote_index(
            &device.id,
            vec![entry("收到的/a.jpg", 1), entry("项目/b.rs", 2)],
        )
        .await
        .unwrap();
    assert_eq!(store.list_remote_index().await.unwrap().len(), 2);

    // 再次交换：整体替换（旧条目消失）
    store
        .replace_remote_index(&device.id, vec![entry("收到的/a.jpg", 9)])
        .await
        .unwrap();
    let idx = store.list_remote_index().await.unwrap();
    assert_eq!(idx.len(), 1);
    assert_eq!(idx[0].size, 9);

    // 降级清理
    store.clear_remote_index(&device.id).await.unwrap();
    assert!(store.list_remote_index().await.unwrap().is_empty());

    // 解除配对级联清空
    store
        .replace_remote_index(&device.id, vec![entry("收到的/a.jpg", 1)])
        .await
        .unwrap();
    store.remove_device(&device.id).await.unwrap();
    assert!(store.list_remote_index().await.unwrap().is_empty());
}

#[tokio::test]
async fn conflicts_replace_diffs_and_preserves_created_at() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    // 首次探测：报告.pdf 两个版本 + 图.png 两个版本
    store
        .replace_conflicts(vec![
            ("收到的/报告.pdf".into(), "h1".into()),
            ("收到的/报告.pdf".into(), "h2".into()),
            ("图.png".into(), "hA".into()),
            ("图.png".into(), "hB".into()),
        ])
        .await
        .unwrap();
    let first = store.list_conflicts().await.unwrap();
    assert_eq!(first.len(), 4);
    let ts = first.iter().find(|c| c.hash == "h1").unwrap().created_at;
    assert!(ts > 0);
    assert_eq!(first[0].status, "open");

    // 再次探测：报告.pdf 冲突仍在（h1 应保留原 created_at），图.png 已解决（消失）
    store
        .replace_conflicts(vec![
            ("收到的/报告.pdf".into(), "h1".into()),
            ("收到的/报告.pdf".into(), "h2".into()),
        ])
        .await
        .unwrap();
    let second = store.list_conflicts().await.unwrap();
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|c| c.rel_path == "收到的/报告.pdf"));
    assert_eq!(
        second.iter().find(|c| c.hash == "h1").unwrap().created_at,
        ts,
        "created_at preserved across replace"
    );

    // 全部解决：清空
    store.replace_conflicts(vec![]).await.unwrap();
    assert!(store.list_conflicts().await.unwrap().is_empty());
}

#[tokio::test]
async fn rejects_invalid_enum_on_insert() {
    // CHECK 约束防御：直接写非法状态会被数据库拒绝
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("aa4c.db");
    let store = Store::open(&db_path).await.unwrap();
    drop(store);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let result = conn.execute(
        "INSERT INTO devices (id, name, platform, public_key, trusted, created_at, updated_at)
         VALUES ('x', 'x', 'freebsd', x'00', 0, 0, 0)",
        [],
    );
    assert!(result.is_err());
}

#[tokio::test]
async fn share_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();

    let share = store
        .insert_share("tok123", "shared/doc.txt", Some(9_999_999_999_999))
        .await
        .unwrap();
    assert_eq!(share.status, "open");
    assert_eq!(share.permission, "read");
    assert_eq!(share.link, ""); // Store 不知道怎么拼链接，留给 Core

    let listed = store.list_shares().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, share.id);

    let fetched = store.get_share_by_token("tok123").await.unwrap().unwrap();
    assert_eq!(fetched.id, share.id);
    assert_eq!(fetched.rel_path, "shared/doc.txt");

    assert!(store
        .get_share_by_token("does-not-exist")
        .await
        .unwrap()
        .is_none());

    // 访问记录
    store
        .record_share_access(&share.id, Some("peer-a"), "download")
        .await
        .unwrap();
    store
        .record_share_access(&share.id, None, "list")
        .await
        .unwrap();
    let access = store.list_share_access(&share.id).await.unwrap();
    assert_eq!(access.len(), 2);
    assert!(access
        .iter()
        .any(|a| a.peer_id.as_deref() == Some("peer-a")));
    assert!(access.iter().any(|a| a.peer_id.is_none()));

    // 吊销：状态变了，记录还在
    store.revoke_share(&share.id).await.unwrap();
    let revoked = store.get_share_by_token("tok123").await.unwrap().unwrap();
    assert_eq!(revoked.status, "revoked");
    assert_eq!(store.list_share_access(&share.id).await.unwrap().len(), 2);

    // 吊销不存在的 id 报错
    assert!(store.revoke_share("nope").await.is_err());
}
