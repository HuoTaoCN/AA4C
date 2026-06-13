//! AA4C 持久化：SQLite 元数据存储与迁移。
//!
//! 表结构见 DATABASE_SCHEMA.md，接口契约见 API_DESIGN.md §7。
//!
//! rusqlite 连接不跨线程共享：内部用单一专职线程持有连接，
//! 通过 channel 把闭包送进线程执行，包装为 async 接口（DATABASE_SCHEMA.md §6）。

#![forbid(unsafe_code)]

mod migrate;
mod record;

use std::path::Path;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use aa4c_types::{Aa4cError, DeviceId, Result, TaskId, TransferFile, TransferStatus, TransferTask};
use rusqlite::{params, Connection, OptionalExtension};

use migrate::db_err;
pub use record::DeviceRecord;

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// SQLite 存储句柄。可廉价 Clone；所有句柄共享同一条专职连接线程。
#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Job>,
}

impl Store {
    /// 打开数据库并自动执行迁移（PRAGMA user_version）。
    pub async fn open(db_path: &Path) -> Result<Self> {
        let path = db_path.to_path_buf();
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<()>>();

        std::thread::Builder::new()
            .name("aa4c-store".into())
            .spawn(move || {
                let mut conn = match open_and_migrate(&path) {
                    Ok(conn) => {
                        let _ = ready_tx.send(Ok(()));
                        conn
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                // 所有 Store 句柄都被 drop 后，recv 报错，线程随之退出
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })?;

        ready_rx
            .await
            .map_err(|_| Aa4cError::Db("store thread exited during init".into()))??;
        Ok(Self { tx })
    }

    /// 在专职线程上执行闭包并等待结果。
    async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |conn| {
                let _ = tx.send(f(conn));
            }))
            .map_err(|_| Aa4cError::Db("store thread terminated".into()))?;
        rx.await
            .map_err(|_| Aa4cError::Db("store thread dropped reply".into()))?
    }

    // —— 设备 ——

    /// 插入或更新设备。`created_at` 仅在首次插入时写入，`updated_at` 总是刷新。
    pub async fn upsert_device(&self, d: &DeviceRecord) -> Result<()> {
        let d = d.clone();
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO devices
                   (id, name, platform, public_key, trusted,
                    paired_at, last_seen_at, last_addr, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   platform = excluded.platform,
                   public_key = excluded.public_key,
                   trusted = excluded.trusted,
                   paired_at = excluded.paired_at,
                   last_seen_at = excluded.last_seen_at,
                   last_addr = excluded.last_addr,
                   updated_at = excluded.updated_at",
                params![
                    d.id,
                    d.name,
                    d.platform.as_str(),
                    d.public_key,
                    d.trusted,
                    d.paired_at,
                    d.last_seen_at,
                    d.last_addr,
                    now,
                ],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    pub async fn get_device(&self, id: &DeviceId) -> Result<Option<DeviceRecord>> {
        let id = id.clone();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, name, platform, public_key, trusted,
                        paired_at, last_seen_at, last_addr, created_at, updated_at
                 FROM devices WHERE id = ?1",
                params![id],
                row_to_device,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    pub async fn list_paired_devices(&self) -> Result<Vec<DeviceRecord>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, platform, public_key, trusted,
                            paired_at, last_seen_at, last_addr, created_at, updated_at
                     FROM devices WHERE trusted = 1 ORDER BY name",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_device)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 删除设备（解除配对）。级联删除其传输任务与文件明细。
    pub async fn remove_device(&self, id: &DeviceId) -> Result<()> {
        let id = id.clone();
        self.call(move |conn| {
            conn.execute("DELETE FROM devices WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    // —— 传输任务 ——

    /// 插入任务及其文件明细（单事务）。
    pub async fn insert_task(&self, t: &TransferTask) -> Result<()> {
        let t = t.clone();
        self.call(move |conn| {
            let now = now_ms();
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute(
                "INSERT INTO transfer_tasks
                   (id, direction, peer_device_id, status, total_bytes,
                    transferred_bytes, file_count, save_dir, error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10)",
                params![
                    t.id,
                    t.direction.as_str(),
                    t.peer,
                    t.status.as_str(),
                    i64::try_from(t.total_bytes).unwrap_or(i64::MAX),
                    i64::try_from(t.transferred_bytes).unwrap_or(i64::MAX),
                    t.files.len(),
                    t.error,
                    t.created_at,
                    now,
                ],
            )
            .map_err(db_err)?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO transfer_files
                           (task_id, file_index, rel_path, size, hash, status)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .map_err(db_err)?;
                for (index, f) in t.files.iter().enumerate() {
                    stmt.execute(params![
                        t.id,
                        index,
                        f.rel_path,
                        i64::try_from(f.size).unwrap_or(i64::MAX),
                        f.hash,
                        f.status.as_str(),
                    ])
                    .map_err(db_err)?;
                }
            }
            tx.commit().map_err(db_err)?;
            Ok(())
        })
        .await
    }

    pub async fn update_task_status(
        &self,
        id: &TaskId,
        status: TransferStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let id = id.clone();
        let error = error.map(str::to_owned);
        self.call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE transfer_tasks
                     SET status = ?2, error = ?3, updated_at = ?4 WHERE id = ?1",
                    params![id, status.as_str(), error, now_ms()],
                )
                .map_err(db_err)?;
            if n == 0 {
                return Err(Aa4cError::Db(format!("task not found: {id}")));
            }
            Ok(())
        })
        .await
    }

    pub async fn update_task_progress(&self, id: &TaskId, transferred: u64) -> Result<()> {
        let id = id.clone();
        self.call(move |conn| {
            conn.execute(
                "UPDATE transfer_tasks
                 SET transferred_bytes = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, i64::try_from(transferred).unwrap_or(i64::MAX), now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 启动清理：把上次运行遗留的未完成任务（等待确认 / 传输中）标记为失败。
    /// 返回被改写的任务数。
    pub async fn fail_incomplete_tasks(&self) -> Result<u64> {
        self.call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE transfer_tasks
                     SET status = ?1, error = ?2, updated_at = ?3
                     WHERE status IN (?4, ?5)",
                    params![
                        TransferStatus::Failed.as_str(),
                        "应用已重启，任务中断",
                        now_ms(),
                        TransferStatus::WaitingAccept.as_str(),
                        TransferStatus::Transferring.as_str(),
                    ],
                )
                .map_err(db_err)?;
            Ok(n as u64)
        })
        .await
    }

    /// 按创建时间倒序分页列出任务（含文件明细）。
    pub async fn list_tasks(&self, limit: u32, offset: u32) -> Result<Vec<TransferTask>> {
        self.call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, direction, peer_device_id, status,
                            total_bytes, transferred_bytes, error, created_at
                     FROM transfer_tasks
                     ORDER BY created_at DESC, id LIMIT ?1 OFFSET ?2",
                )
                .map_err(db_err)?;
            let mut tasks = stmt
                .query_map(params![limit, offset], row_to_task)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;

            let mut file_stmt = conn
                .prepare(
                    "SELECT rel_path, size, hash, status
                     FROM transfer_files WHERE task_id = ?1 ORDER BY file_index",
                )
                .map_err(db_err)?;
            for task in &mut tasks {
                task.files = file_stmt
                    .query_map(params![task.id], row_to_file)
                    .map_err(db_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db_err)?;
            }
            Ok(tasks)
        })
        .await
    }

    // —— 设置 ——

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let key = key.to_owned();
        self.call(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let key = key.to_owned();
        let value = value.to_owned();
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                   value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }
}

fn open_and_migrate(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path).map_err(db_err)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(db_err)?;
    migrate::migrate(&mut conn)?;
    Ok(conn)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    Ok(DeviceRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        platform: parse_col(row, 2)?,
        public_key: row.get(3)?,
        trusted: row.get(4)?,
        paired_at: row.get(5)?,
        last_seen_at: row.get(6)?,
        last_addr: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferTask> {
    Ok(TransferTask {
        id: row.get(0)?,
        direction: parse_col(row, 1)?,
        peer: row.get(2)?,
        files: Vec::new(), // 由 list_tasks 第二步填充
        status: parse_col(row, 3)?,
        total_bytes: get_u64(row, 4)?,
        transferred_bytes: get_u64(row, 5)?,
        created_at: row.get(7)?,
        error: row.get(6)?,
    })
}

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferFile> {
    Ok(TransferFile {
        rel_path: row.get(0)?,
        size: get_u64(row, 1)?,
        hash: row.get(2)?,
        status: parse_col(row, 3)?,
    })
}

/// 读取 TEXT 列并 FromStr 解析为枚举；非法值视为列类型错误。
fn parse_col<T: std::str::FromStr>(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    s.parse().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("invalid enum value: {s}").into(),
        )
    })
}

fn get_u64(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<u64> {
    let v: i64 = row.get(idx)?;
    Ok(u64::try_from(v).unwrap_or(0))
}
