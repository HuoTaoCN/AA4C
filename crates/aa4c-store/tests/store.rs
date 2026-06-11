//! aa4c-store 集成测试（V0.1_IMPLEMENTATION_PLAN.md M1）。

use aa4c_store::{DeviceRecord, Store};
use aa4c_types::{Direction, FileStatus, Platform, TransferFile, TransferStatus, TransferTask};

fn sample_device(id: &str, trusted: bool) -> DeviceRecord {
    DeviceRecord {
        id: id.repeat(64 / id.len().max(1)),
        name: format!("设备-{id}"),
        platform: Platform::Macos,
        public_key: vec![7u8; 32],
        trusted,
        paired_at: trusted.then_some(1_750_000_000_000),
        last_seen_at: Some(1_750_000_000_000),
        last_addr: Some("192.168.1.10:42420".into()),
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
    assert_eq!(version, 1);
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
    assert!(got.created_at > 0);
    assert_eq!(got.created_at, got.updated_at);

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
