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

use aa4c_types::{
    Aa4cError, ArchiveCategory, ArchiveEntry, ArchiveLogEntry, ArchiveRule, ArchiveTag, DeviceId,
    DownloadKind, DownloadOptions, DownloadStatus, DownloadTask, KbDocStatus, KbDocument, KbSource,
    KbSourceSummary, ModelMeta, RemoteIndexEntry, Result, ScopeKind, Share, ShareAccess,
    SyncConflict, SyncFileEntry, SyncScope, TagSource, TaskId, TransferFile, TransferStatus,
    TransferTask, TrustLevel,
};
use rusqlite::{params, Connection, OptionalExtension};

use migrate::db_err;
pub use record::DeviceRecord;

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// 一条知识库 chunk 连同它所属文档/来源的路径信息（`list_kb_chunks_for_search`
/// 的返回行）——不进 `aa4c-types`：带 embedding 向量的行从不跨越 Command 边界，
/// 只在 `aa4c-store`/`aa4c-ai::kb` 之间传递（`aa4c-core` 转手时按需只取路径）。
#[derive(Debug, Clone)]
pub struct KbChunkRow {
    pub doc_id: String,
    pub source_path: String,
    pub rel_path: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

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
                   (id, name, platform, public_key, trusted, trust_level,
                    paired_at, last_seen_at, last_addr, server_hint, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   platform = excluded.platform,
                   public_key = excluded.public_key,
                   trusted = excluded.trusted,
                   trust_level = excluded.trust_level,
                   paired_at = excluded.paired_at,
                   last_seen_at = excluded.last_seen_at,
                   last_addr = excluded.last_addr,
                   server_hint = excluded.server_hint,
                   updated_at = excluded.updated_at",
                params![
                    d.id,
                    d.name,
                    d.platform.as_str(),
                    d.public_key,
                    d.trusted,
                    d.trust_level.as_str(),
                    d.paired_at,
                    d.last_seen_at,
                    d.last_addr,
                    d.server_hint,
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
                        paired_at, last_seen_at, last_addr, created_at, updated_at,
                        trust_level, server_hint
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
                            paired_at, last_seen_at, last_addr, created_at, updated_at,
                            trust_level, server_hint
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

    /// 变更设备信任分级（完全信任 ⇄ 朋友）。设备不存在时报错。
    pub async fn set_trust_level(&self, id: &DeviceId, level: TrustLevel) -> Result<()> {
        let id = id.clone();
        self.call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE devices SET trust_level = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, level.as_str(), now_ms()],
                )
                .map_err(db_err)?;
            if n == 0 {
                return Err(Aa4cError::DeviceNotFound(id));
            }
            Ok(())
        })
        .await
    }

    /// 更新对端 home server 地址（CONNECT_DESIGN.md §3.4，里程碑 C2）。设备不存在时报错。
    pub async fn set_server_hint(&self, id: &DeviceId, server_hint: Option<String>) -> Result<()> {
        let id = id.clone();
        self.call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE devices SET server_hint = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, server_hint, now_ms()],
                )
                .map_err(db_err)?;
            if n == 0 {
                return Err(Aa4cError::DeviceNotFound(id));
            }
            Ok(())
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

    // —— 同步范围 + 本地文件索引（SYNC_DESIGN.md §3/§6，DATABASE_SCHEMA.md §4.2-4.3）——

    /// 新建一个共享范围；按 `local_path` 去重（已存在则原样返回）。
    pub async fn upsert_sync_scope(&self, kind: ScopeKind, local_path: &str) -> Result<SyncScope> {
        let local_path = local_path.to_owned();
        self.call(move |conn| {
            if let Some(existing) = conn
                .query_row(
                    "SELECT id, kind, local_path, created_at FROM sync_scopes WHERE local_path = ?1",
                    params![local_path],
                    row_to_scope,
                )
                .optional()
                .map_err(db_err)?
            {
                return Ok(existing);
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = now_ms();
            conn.execute(
                "INSERT INTO sync_scopes (id, kind, local_path, mode, created_at)
                 VALUES (?1, ?2, ?3, 'ondemand', ?4)",
                params![id, kind.as_str(), local_path, now],
            )
            .map_err(db_err)?;
            Ok(SyncScope {
                id,
                kind,
                local_path,
                created_at: now,
            })
        })
        .await
    }

    /// 确保 Inbox 范围存在并指向 `local_path`（全局唯一一个）；路径变化时原地更新——
    /// 旧路径下的条目会在下次扫描时随 `replace_scope_index` 一起被清空。
    pub async fn ensure_inbox_scope(&self, local_path: &str) -> Result<SyncScope> {
        let local_path = local_path.to_owned();
        self.call(move |conn| {
            let existing: Option<SyncScope> = conn
                .query_row(
                    "SELECT id, kind, local_path, created_at FROM sync_scopes WHERE kind = 'inbox'",
                    [],
                    row_to_scope,
                )
                .optional()
                .map_err(db_err)?;
            if let Some(mut scope) = existing {
                if scope.local_path != local_path {
                    conn.execute(
                        "UPDATE sync_scopes SET local_path = ?2 WHERE id = ?1",
                        params![scope.id, local_path],
                    )
                    .map_err(db_err)?;
                    scope.local_path = local_path;
                }
                return Ok(scope);
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = now_ms();
            conn.execute(
                "INSERT INTO sync_scopes (id, kind, local_path, mode, created_at)
                 VALUES (?1, 'inbox', ?2, 'ondemand', ?3)",
                params![id, local_path, now],
            )
            .map_err(db_err)?;
            Ok(SyncScope {
                id,
                kind: ScopeKind::Inbox,
                local_path,
                created_at: now,
            })
        })
        .await
    }

    pub async fn list_sync_scopes(&self) -> Result<Vec<SyncScope>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, local_path, created_at FROM sync_scopes ORDER BY created_at",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_scope)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 删除一个共享范围（级联删除其索引条目）。
    pub async fn remove_sync_scope(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |conn| {
            conn.execute("DELETE FROM sync_scopes WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 某范围当前索引（扫描时用于和文件系统比对 mtime/size，决定是否重新哈希）。
    pub async fn list_scope_index(&self, scope_id: &str) -> Result<Vec<SyncFileEntry>> {
        let scope_id = scope_id.to_owned();
        self.call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT scope_id, rel_path, size, mtime, hash, present_local
                     FROM sync_file_index WHERE scope_id = ?1",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![scope_id], row_to_sync_file_entry)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 全部范围的索引并集（统一视图）。
    pub async fn list_all_sync_files(&self) -> Result<Vec<SyncFileEntry>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT scope_id, rel_path, size, mtime, hash, present_local
                     FROM sync_file_index ORDER BY rel_path",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_sync_file_entry)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 用一次扫描结果整体替换某范围的索引：删除扫描中已消失的条目，插入/更新现存条目（单事务）。
    pub async fn replace_scope_index(
        &self,
        scope_id: &str,
        entries: Vec<SyncFileEntry>,
    ) -> Result<()> {
        let scope_id = scope_id.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            let keep: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.rel_path.as_str()).collect();
            {
                let mut stmt = tx
                    .prepare("SELECT rel_path FROM sync_file_index WHERE scope_id = ?1")
                    .map_err(db_err)?;
                let existing: Vec<String> = stmt
                    .query_map(params![scope_id], |r| r.get(0))
                    .map_err(db_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db_err)?;
                let mut del = tx
                    .prepare("DELETE FROM sync_file_index WHERE scope_id = ?1 AND rel_path = ?2")
                    .map_err(db_err)?;
                for rel in existing {
                    if !keep.contains(rel.as_str()) {
                        del.execute(params![scope_id, rel]).map_err(db_err)?;
                    }
                }
            }
            let now = now_ms();
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO sync_file_index
                           (scope_id, rel_path, size, mtime, hash, present_local, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(scope_id, rel_path) DO UPDATE SET
                           size = excluded.size,
                           mtime = excluded.mtime,
                           hash = excluded.hash,
                           present_local = excluded.present_local,
                           updated_at = excluded.updated_at",
                    )
                    .map_err(db_err)?;
                for e in &entries {
                    stmt.execute(params![
                        scope_id,
                        e.rel_path,
                        i64::try_from(e.size).unwrap_or(i64::MAX),
                        e.mtime,
                        e.hash,
                        e.present_local,
                        now,
                    ])
                    .map_err(db_err)?;
                }
            }
            tx.commit().map_err(db_err)?;
            Ok(())
        })
        .await
    }

    // —— 远端索引（跨设备摘要交换，SYNC_DESIGN.md §3.3，DATABASE_SCHEMA.md §4.4，里程碑 3）——

    /// 用一次交换收到的完整快照整体替换某设备的远端索引（单事务先清后插）。
    pub async fn replace_remote_index(
        &self,
        device_id: &str,
        entries: Vec<RemoteIndexEntry>,
    ) -> Result<()> {
        let device_id = device_id.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute(
                "DELETE FROM remote_index WHERE device_id = ?1",
                params![device_id],
            )
            .map_err(db_err)?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO remote_index (device_id, rel_path, size, hash, seen_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(device_id, rel_path) DO UPDATE SET
                           size = excluded.size, hash = excluded.hash, seen_at = excluded.seen_at",
                    )
                    .map_err(db_err)?;
                for e in &entries {
                    stmt.execute(params![
                        device_id,
                        e.rel_path,
                        i64::try_from(e.size).unwrap_or(i64::MAX),
                        e.hash,
                        e.seen_at,
                    ])
                    .map_err(db_err)?;
                }
            }
            tx.commit().map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 清空某设备的远端索引（完全信任降级为朋友时调用，SYNC_DESIGN.md §2）。
    pub async fn clear_remote_index(&self, device_id: &str) -> Result<()> {
        let device_id = device_id.to_owned();
        self.call(move |conn| {
            conn.execute(
                "DELETE FROM remote_index WHERE device_id = ?1",
                params![device_id],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 全部远端索引条目（统一视图归并用）。
    pub async fn list_remote_index(&self) -> Result<Vec<RemoteIndexEntry>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT device_id, rel_path, size, hash, seen_at
                     FROM remote_index ORDER BY rel_path",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_remote_entry)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    // —— 冲突记录（同名不同 hash，SYNC_DESIGN.md §8，DATABASE_SCHEMA.md §4.5，里程碑 5）——

    /// 用当前统一视图探测到的冲突版本整体替换冲突表（单事务 diff：删除已消失的、
    /// 保留仍在的 `created_at`、插入新出现的）。`versions` 为 (rel_path, hash) 列表。
    pub async fn replace_conflicts(&self, versions: Vec<(String, String)>) -> Result<()> {
        self.call(move |conn| {
            let now = now_ms();
            let keep: std::collections::HashSet<(String, String)> =
                versions.iter().cloned().collect();
            let tx = conn.transaction().map_err(db_err)?;
            {
                let existing: Vec<(String, String)> = {
                    let mut stmt = tx
                        .prepare("SELECT rel_path, hash FROM sync_conflicts")
                        .map_err(db_err)?;
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                        .map_err(db_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(db_err)?;
                    rows
                };
                for (rel_path, hash) in existing.iter().filter(|k| !keep.contains(k)) {
                    tx.execute(
                        "DELETE FROM sync_conflicts WHERE rel_path = ?1 AND hash = ?2",
                        params![rel_path, hash],
                    )
                    .map_err(db_err)?;
                }
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO sync_conflicts (rel_path, hash, status, created_at)
                         VALUES (?1, ?2, 'open', ?3)
                         ON CONFLICT(rel_path, hash) DO NOTHING",
                    )
                    .map_err(db_err)?;
                for (rel_path, hash) in &versions {
                    stmt.execute(params![rel_path, hash, now]).map_err(db_err)?;
                }
            }
            tx.commit().map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 全部冲突版本行（按 rel_path 排序；一个 rel_path 有 ≥2 行即一处冲突）。
    pub async fn list_conflicts(&self) -> Result<Vec<SyncConflict>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT rel_path, hash, status, created_at
                     FROM sync_conflicts ORDER BY rel_path, hash",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(SyncConflict {
                        rel_path: r.get(0)?,
                        hash: r.get(1)?,
                        status: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                })
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    // —— 分享链接（CONNECT_DESIGN.md §7/§8，里程碑 C6）——

    /// 新建一条分享记录（`id`/`created_at` 由 Store 生成）。`token` 唯一性由调用方保证
    /// （Core 生成时已有足够熵，冲突概率可忽略不计）；`UNIQUE` 约束仍在，真撞上会报错。
    pub async fn insert_share(
        &self,
        token: &str,
        rel_path: &str,
        expires_at: Option<i64>,
    ) -> Result<Share> {
        let token = token.to_string();
        let rel_path = rel_path.to_string();
        self.call(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            let now = now_ms();
            conn.execute(
                "INSERT INTO shares (id, token, rel_path, permission, expires_at, status, created_at)
                 VALUES (?1, ?2, ?3, 'read', ?4, 'open', ?5)",
                params![id, token, rel_path, expires_at, now],
            )
            .map_err(db_err)?;
            Ok(Share {
                id,
                token,
                rel_path,
                permission: "read".to_string(),
                expires_at,
                status: "open".to_string(),
                created_at: now,
                link: String::new(),
            })
        })
        .await
    }

    /// 按创建时间倒序列出全部分享记录。`link` 字段留空——完整链接需要本机 device_id +
    /// 当前配置的服务器地址（`aa4c_types::Share` 文档），由调用方（`aa4c-core`）现算。
    pub async fn list_shares(&self) -> Result<Vec<Share>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, token, rel_path, permission, expires_at, status, created_at
                     FROM shares ORDER BY created_at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_share)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 按 token 查一条分享记录（供打开分享链接的一方校验用）。**不区分**「不存在」/
    /// 「已吊销」/「已过期」——统一回 `None`，调用方按同一套「不泄露拒绝原因」的惯例
    /// 处理（同服务器 Lookup 对未注册/不在允许名单的处理，见 `aa4c-server`）。
    pub async fn get_share_by_token(&self, token: &str) -> Result<Option<Share>> {
        let token = token.to_string();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, token, rel_path, permission, expires_at, status, created_at
                 FROM shares WHERE token = ?1",
                params![token],
                row_to_share,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    /// 吊销一条分享：置 `status='revoked'`，保留记录供审计（CONNECT_DESIGN.md §7.1）。
    pub async fn revoke_share(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            let n = conn
                .execute(
                    "UPDATE shares SET status = 'revoked' WHERE id = ?1",
                    params![id],
                )
                .map_err(db_err)?;
            if n == 0 {
                return Err(Aa4cError::Db(format!("share not found: {id}")));
            }
            Ok(())
        })
        .await
    }

    /// 记一条访问记录（可选功能，供「查看访问记录」，DATABASE_SCHEMA.md §4c.2）。
    pub async fn record_share_access(
        &self,
        share_id: &str,
        peer_id: Option<&str>,
        action: &str,
    ) -> Result<()> {
        let share_id = share_id.to_string();
        let peer_id = peer_id.map(str::to_owned);
        let action = action.to_string();
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO share_access (share_id, peer_id, action, at) VALUES (?1, ?2, ?3, ?4)",
                params![share_id, peer_id, action, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 某条分享的全部访问记录（按时间倒序）。
    pub async fn list_share_access(&self, share_id: &str) -> Result<Vec<ShareAccess>> {
        let share_id = share_id.to_string();
        self.call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, share_id, peer_id, action, at
                     FROM share_access WHERE share_id = ?1 ORDER BY at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![share_id], row_to_share_access)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    // —— 下载任务（DOWNLOAD_DESIGN.md §4/§9，里程碑 D1）——

    /// 新建一条下载任务。`id` 由调用方传入（引擎原生任务号，如 aria2 GID），
    /// 不由 Store 生成——这是"GID 直接当任务 id"决定的直接体现。
    /// `options` 为 `None` 或全空时 `options` 列写 NULL——绝大多数任务没有任何
    /// 自定义选项，不该在库里留一行 `{}`（迁移 011）。
    pub async fn insert_download(
        &self,
        id: &str,
        kind: DownloadKind,
        url: &str,
        options: Option<&DownloadOptions>,
    ) -> Result<DownloadTask> {
        let id = id.to_string();
        let url = url.to_string();
        let options = options.filter(|o| !o.is_empty()).cloned();
        self.call(move |conn| {
            let now = now_ms();
            let options_json =
                match &options {
                    Some(o) => Some(serde_json::to_string(o).map_err(|e| {
                        Aa4cError::Db(format!("encode download options failed: {e}"))
                    })?),
                    None => None,
                };
            conn.execute(
                "INSERT INTO download_tasks
                   (id, kind, url, status, total_bytes, downloaded_bytes, created_at, updated_at,
                    options)
                 VALUES (?1, ?2, ?3, 'waiting', 0, 0, ?4, ?4, ?5)",
                params![id, kind.as_str(), url, now, options_json],
            )
            .map_err(db_err)?;
            Ok(DownloadTask {
                id,
                kind,
                url,
                save_path: None,
                status: DownloadStatus::Waiting,
                total_bytes: 0,
                downloaded_bytes: 0,
                error: None,
                created_at: now,
                options,
            })
        })
        .await
    }

    pub async fn get_download(&self, id: &str) -> Result<Option<DownloadTask>> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, kind, url, save_path, status, total_bytes, downloaded_bytes,
                        error, created_at, options
                 FROM download_tasks WHERE id = ?1",
                params![id],
                row_to_download,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    /// 按创建时间倒序列出全部下载任务。
    pub async fn list_downloads(&self) -> Result<Vec<DownloadTask>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, url, save_path, status, total_bytes, downloaded_bytes,
                            error, created_at, options
                     FROM download_tasks ORDER BY created_at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_download)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 未完成任务（active/waiting/paused）——供启动/WS 重连后的对账使用
    /// （DOWNLOAD_DESIGN.md §3.4）。
    pub async fn list_unfinished_downloads(&self) -> Result<Vec<DownloadTask>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, url, save_path, status, total_bytes, downloaded_bytes,
                            error, created_at, options
                     FROM download_tasks
                     WHERE status IN ('active','waiting','paused')
                     ORDER BY created_at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_download)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 状态迁移（含可选的失败原因/落盘路径回填）——对应 §4 的"状态迁移必写"。
    pub async fn update_download_status(
        &self,
        id: &str,
        status: DownloadStatus,
        error: Option<&str>,
        save_path: Option<&str>,
    ) -> Result<()> {
        let id = id.to_string();
        let error = error.map(str::to_owned);
        let save_path = save_path.map(str::to_owned);
        self.call(move |conn| {
            conn.execute(
                "UPDATE download_tasks
                 SET status = ?2, error = ?3,
                     save_path = COALESCE(?4, save_path),
                     updated_at = ?5
                 WHERE id = ?1",
                params![id, status.as_str(), error, save_path, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 进度更新（调用方负责节流，Store 不管频率，见 §4）。
    pub async fn update_download_progress(
        &self,
        id: &str,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute(
                "UPDATE download_tasks
                 SET downloaded_bytes = ?2, total_bytes = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![
                    id,
                    i64::try_from(downloaded_bytes).unwrap_or(i64::MAX),
                    i64::try_from(total_bytes).unwrap_or(i64::MAX),
                    now_ms()
                ],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 硬删指定 id 的下载任务行（不是软标记 `removed`）——调用方（`clear_completed`，
    /// DOWNLOAD_DESIGN.md §9 批量操作）已经先让引擎"忘记"了这些任务，记录留着
    /// 没有意义。接收显式 id 列表而不是"删所有 complete 状态"的隐式全表操作，
    /// 因为调用方要先对每个 id 发引擎侧调用、只删成功的那些，两步不该合并成一步
    /// 黑盒 SQL。逐条删除而不是拼 `IN (?,?,...)`——批量清除不是热路径，简单优先。
    pub async fn delete_completed_downloads(&self, ids: &[String]) -> Result<()> {
        let ids = ids.to_vec();
        self.call(move |conn| {
            for id in &ids {
                conn.execute("DELETE FROM download_tasks WHERE id = ?1", params![id])
                    .map_err(db_err)?;
            }
            Ok(())
        })
        .await
    }

    /// 归档把文件挪走后回填新路径——不更新的话「打开所在文件夹」会指向空位
    /// （ARCHIVE_DESIGN.md §2.4）。任务不存在（已被批量清除等）时静默跳过，同
    /// `update_download_status` 一贯的"不强行要求调用方先查存在性"风格。
    pub async fn update_download_save_path(&self, id: &str, new_path: &str) -> Result<()> {
        let id = id.to_string();
        let new_path = new_path.to_string();
        self.call(move |conn| {
            conn.execute(
                "UPDATE download_tasks SET save_path = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, new_path, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    // —— 归档（ARCHIVE_DESIGN.md §2/§4，里程碑 AI1）——

    /// 插入或更新一条归档规则（`upsert`：`id` 由调用方生成，新建走 core 侧 uuid）。
    /// `created_at` 仅首次插入写入，`updated_at` 总是刷新为当前时间（同 `upsert_device`
    /// 的既有模式），不信调用方传入的时间戳，避免误传旧值。
    pub async fn upsert_archive_rule(&self, rule: &ArchiveRule) -> Result<()> {
        let id = rule.id.clone();
        let name = rule.name.clone();
        let enabled = rule.enabled;
        let position = rule.position;
        let match_json =
            serde_json::to_string(&rule.matcher).map_err(|e| Aa4cError::Db(e.to_string()))?;
        let action_json =
            serde_json::to_string(&rule.action).map_err(|e| Aa4cError::Db(e.to_string()))?;
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO archive_rules
                   (id, name, enabled, position, match_json, action_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   enabled = excluded.enabled,
                   position = excluded.position,
                   match_json = excluded.match_json,
                   action_json = excluded.action_json,
                   updated_at = excluded.updated_at",
                params![id, name, enabled, position, match_json, action_json, now],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    pub async fn delete_archive_rule(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute("DELETE FROM archive_rules WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 按 `position` 升序列出全部规则（匹配时取第一条命中的，见 ARCHIVE_DESIGN §2.3）。
    pub async fn list_archive_rules(&self) -> Result<Vec<ArchiveRule>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, enabled, position, match_json, action_json,
                            created_at, updated_at
                     FROM archive_rules ORDER BY position ASC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_archive_rule)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 新建一条归档记录（文件被规则或手动归档移动之后）。`id` 由调用方生成
    /// （core 侧 uuid，同 `insert_download` 的"id 由调用方决定"惯例，只是这里没有
    /// 引擎原生 id 可复用）。
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_archive_entry(
        &self,
        id: &str,
        current_path: &str,
        category: ArchiveCategory,
        size: u64,
        model_meta: Option<&ModelMeta>,
    ) -> Result<ArchiveEntry> {
        let id = id.to_string();
        let current_path = current_path.to_string();
        let model_meta_json = model_meta
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Aa4cError::Db(e.to_string()))?;
        let model_meta_owned = model_meta.cloned();
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO archive_entries
                   (id, current_path, category, size, model_meta_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    id,
                    current_path,
                    category.as_str(),
                    i64::try_from(size).unwrap_or(i64::MAX),
                    model_meta_json,
                    now,
                ],
            )
            .map_err(db_err)?;
            Ok(ArchiveEntry {
                id,
                current_path,
                category,
                size,
                model_meta: model_meta_owned,
                created_at: now,
                updated_at: now,
            })
        })
        .await
    }

    pub async fn get_archive_entry(&self, id: &str) -> Result<Option<ArchiveEntry>> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, current_path, category, size, model_meta_json,
                        created_at, updated_at
                 FROM archive_entries WHERE id = ?1",
                params![id],
                row_to_archive_entry,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    /// 按创建时间倒序列出全部归档记录（同 `list_downloads` 的既有排序惯例）。
    pub async fn list_archive_entries(&self) -> Result<Vec<ArchiveEntry>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, current_path, category, size, model_meta_json,
                            created_at, updated_at
                     FROM archive_entries ORDER BY created_at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_archive_entry)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 撤销把记录的路径改回去时用（正向移动已经在 `insert_archive_entry` 里写过一次）。
    pub async fn update_archive_entry_path(&self, id: &str, new_path: &str) -> Result<()> {
        let id = id.to_string();
        let new_path = new_path.to_string();
        self.call(move |conn| {
            conn.execute(
                "UPDATE archive_entries SET current_path = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, new_path, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    pub async fn delete_archive_entry(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute("DELETE FROM archive_entries WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 追加标签（`INSERT OR IGNORE`——同一 entry 重复打同一个 tag 不报错，幂等）。
    pub async fn add_archive_tags(
        &self,
        entry_id: &str,
        tags: &[(String, TagSource)],
    ) -> Result<()> {
        let entry_id = entry_id.to_string();
        let tags = tags.to_vec();
        self.call(move |conn| {
            for (tag, source) in &tags {
                conn.execute(
                    "INSERT OR IGNORE INTO archive_tags (entry_id, tag, source) VALUES (?1, ?2, ?3)",
                    params![entry_id, tag, source.as_str()],
                )
                .map_err(db_err)?;
            }
            Ok(())
        })
        .await
    }

    /// 撤销时摘掉本次归档追加的标签（ARCHIVE_DESIGN §2.4）。
    pub async fn remove_archive_tag(&self, entry_id: &str, tag: &str) -> Result<()> {
        let entry_id = entry_id.to_string();
        let tag = tag.to_string();
        self.call(move |conn| {
            conn.execute(
                "DELETE FROM archive_tags WHERE entry_id = ?1 AND tag = ?2",
                params![entry_id, tag],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    pub async fn list_archive_tags(&self, entry_id: &str) -> Result<Vec<ArchiveTag>> {
        let entry_id = entry_id.to_string();
        self.call(move |conn| {
            let mut stmt = conn
                .prepare("SELECT entry_id, tag, source FROM archive_tags WHERE entry_id = ?1")
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![entry_id], row_to_archive_tag)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 记一条移动历史，返回带自增 id 的完整记录（撤销要靠这个 id 定位）。
    pub async fn append_archive_log(
        &self,
        entry_id: &str,
        from_path: &str,
        to_path: &str,
        rule_id: Option<&str>,
    ) -> Result<ArchiveLogEntry> {
        let entry_id = entry_id.to_string();
        let from_path = from_path.to_string();
        let to_path = to_path.to_string();
        let rule_id = rule_id.map(str::to_owned);
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO archive_log (entry_id, from_path, to_path, rule_id, at, undone)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                params![entry_id, from_path, to_path, rule_id, now],
            )
            .map_err(db_err)?;
            let id = conn.last_insert_rowid();
            Ok(ArchiveLogEntry {
                id,
                entry_id,
                from_path,
                to_path,
                rule_id,
                at: now,
                undone: false,
            })
        })
        .await
    }

    pub async fn get_archive_log_entry(&self, id: i64) -> Result<Option<ArchiveLogEntry>> {
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, entry_id, from_path, to_path, rule_id, at, undone
                 FROM archive_log WHERE id = ?1",
                params![id],
                row_to_archive_log,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    /// 按时间倒序列出全部移动历史（归档页「最近归档动作」用）。
    pub async fn list_archive_log(&self) -> Result<Vec<ArchiveLogEntry>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, entry_id, from_path, to_path, rule_id, at, undone
                     FROM archive_log ORDER BY at DESC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_archive_log)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    pub async fn mark_archive_log_undone(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            conn.execute(
                "UPDATE archive_log SET undone = 1 WHERE id = ?1",
                params![id],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    // —— 本地知识库（ARCHIVE_DESIGN.md §6/§4h，里程碑 AI4）——

    /// 新增一个知识库来源目录。`id` 由调用方生成（core 侧 uuid，同 `insert_archive_entry`
    /// 的既有模式）。
    pub async fn insert_kb_source(&self, id: &str, path: &str) -> Result<KbSource> {
        let id = id.to_string();
        let path = path.to_string();
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO kb_sources (id, path, created_at) VALUES (?1, ?2, ?3)",
                params![id, path, now],
            )
            .map_err(db_err)?;
            Ok(KbSource {
                id,
                path,
                created_at: now,
            })
        })
        .await
    }

    /// 删除一个来源（级联删除其下全部 `kb_documents`/`kb_chunks`）。
    pub async fn delete_kb_source(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute("DELETE FROM kb_sources WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 按 id 查单个来源（`kb_reindex` 编排要在扫描前先拿到 `path`）。
    pub async fn get_kb_source(&self, id: &str) -> Result<Option<KbSource>> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, path, created_at FROM kb_sources WHERE id = ?1",
                params![id],
                row_to_kb_source,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    pub async fn list_kb_sources(&self) -> Result<Vec<KbSource>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, path, created_at FROM kb_sources ORDER BY created_at ASC")
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], row_to_kb_source)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 每个来源的摄入状态摘要（`kb_list_sources` Command 直接返回这个，前端不用
    /// 再单独查文档列表才能显示进度，见 `KbSourceSummary` 文档）。
    pub async fn list_kb_source_summaries(&self) -> Result<Vec<KbSourceSummary>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, s.path, s.created_at,
                            COUNT(d.id),
                            COALESCE(SUM(d.status = 'indexed'), 0),
                            COALESCE(SUM(d.status = 'failed'), 0)
                     FROM kb_sources s
                     LEFT JOIN kb_documents d ON d.source_id = s.id
                     GROUP BY s.id
                     ORDER BY s.created_at ASC",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(KbSourceSummary {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        created_at: row.get(2)?,
                        doc_count: get_u32(row, 3)?,
                        indexed_count: get_u32(row, 4)?,
                        failed_count: get_u32(row, 5)?,
                    })
                })
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 扫描增量判断用：同一来源下按相对路径查已有文档记录（`None` = 首次见到这个文件）。
    pub async fn get_kb_document_by_rel_path(
        &self,
        source_id: &str,
        rel_path: &str,
    ) -> Result<Option<KbDocument>> {
        let source_id = source_id.to_string();
        let rel_path = rel_path.to_string();
        self.call(move |conn| {
            conn.query_row(
                "SELECT id, source_id, rel_path, mtime, hash, status, updated_at
                 FROM kb_documents WHERE source_id = ?1 AND rel_path = ?2",
                params![source_id, rel_path],
                row_to_kb_document,
            )
            .optional()
            .map_err(db_err)
        })
        .await
    }

    /// 一个来源目录下全部文档（扫描时用来找出"数据库里有、这次扫描没扫到"的
    /// 已删除文件，见 `aa4c-ai::kb` 的增量扫描逻辑）。
    pub async fn list_kb_documents(&self, source_id: &str) -> Result<Vec<KbDocument>> {
        let source_id = source_id.to_string();
        self.call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, source_id, rel_path, mtime, hash, status, updated_at
                     FROM kb_documents WHERE source_id = ?1",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map(params![source_id], row_to_kb_document)
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 插入或更新一条文档记录（`id` 首次插入由调用方生成 uuid，之后按同一 `id`
    /// 更新——调用方在插入前先 `get_kb_document_by_rel_path` 拿到既有 id，同
    /// `upsert_archive_rule` 的既有模式）。
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_kb_document(
        &self,
        id: &str,
        source_id: &str,
        rel_path: &str,
        mtime: i64,
        hash: &str,
        status: KbDocStatus,
    ) -> Result<KbDocument> {
        let id = id.to_string();
        let source_id = source_id.to_string();
        let rel_path = rel_path.to_string();
        let hash = hash.to_string();
        self.call(move |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO kb_documents (id, source_id, rel_path, mtime, hash, status, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   rel_path = excluded.rel_path,
                   mtime = excluded.mtime,
                   hash = excluded.hash,
                   status = excluded.status,
                   updated_at = excluded.updated_at",
                params![id, source_id, rel_path, mtime, hash, status.as_str(), now],
            )
            .map_err(db_err)?;
            Ok(KbDocument {
                id,
                source_id,
                rel_path,
                mtime,
                hash,
                status,
                updated_at: now,
            })
        })
        .await
    }

    /// 摄入成功/失败后单独刷新状态，不用重新传一遍全部字段。
    pub async fn set_kb_document_status(&self, id: &str, status: KbDocStatus) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute(
                "UPDATE kb_documents SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, status.as_str(), now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 删除一条文档记录（级联删除其 `kb_chunks`；文件在磁盘上消失或来源被移除时用）。
    pub async fn delete_kb_document(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.call(move |conn| {
            conn.execute("DELETE FROM kb_documents WHERE id = ?1", params![id])
                .map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 用新分块整体替换一个文档的全部 chunk（重新摄入=先清空旧的再插入新的，
    /// 不做逐条 diff——分块边界随内容变化，没有稳定的"这块对应哪块"关系，同
    /// `finish_move` 一类"整体替换比增量对比更简单也更不容易出 bug"的既有取舍）。
    /// `chunks` 是 `(text, embedding)` 对，`seq` 按切片顺序从 0 开始编号。
    /// 删除+批量插入包在一个事务里，不会出现"删了旧的但新的还没插完"的中间态。
    pub async fn replace_kb_chunks(
        &self,
        doc_id: &str,
        chunks: &[(String, Vec<f32>)],
    ) -> Result<()> {
        let doc_id = doc_id.to_string();
        let chunks = chunks.to_vec();
        self.call(move |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute("DELETE FROM kb_chunks WHERE doc_id = ?1", params![doc_id])
                .map_err(db_err)?;
            for (seq, (text, embedding)) in chunks.iter().enumerate() {
                let dims = i64::try_from(embedding.len()).unwrap_or(i64::MAX);
                let blob = encode_embedding(embedding);
                tx.execute(
                    "INSERT INTO kb_chunks (doc_id, seq, text, embedding, dims)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        doc_id,
                        i64::try_from(seq).unwrap_or(i64::MAX),
                        text,
                        blob,
                        dims
                    ],
                )
                .map_err(db_err)?;
            }
            tx.commit().map_err(db_err)?;
            Ok(())
        })
        .await
    }

    /// 暴力余弦检索用：读出全部 chunk 连同各自文档所属来源的绝对路径拼接件
    /// （`source_path`/`rel_path` 分开返回，由调用方用 `Path::join` 拼——SQL 里
    /// 用 `||` 拼字符串对 Windows 反斜杠路径不安全，见 ARCHIVE_DESIGN §6）。
    /// 个人规模（数万 chunk）一次性读入内存后在 Rust 里算，见 §6 的既定取舍。
    pub async fn list_kb_chunks_for_search(&self) -> Result<Vec<KbChunkRow>> {
        self.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.doc_id, s.path, d.rel_path, c.text, c.embedding
                     FROM kb_chunks c
                     JOIN kb_documents d ON d.id = c.doc_id
                     JOIN kb_sources s ON s.id = d.source_id
                     WHERE d.status = 'indexed'",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], |row| {
                    let embedding_blob: Vec<u8> = row.get(4)?;
                    Ok(KbChunkRow {
                        doc_id: row.get(0)?,
                        source_path: row.get(1)?,
                        rel_path: row.get(2)?,
                        text: row.get(3)?,
                        embedding: decode_embedding(&embedding_blob),
                    })
                })
                .map_err(db_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_err)?;
            Ok(rows)
        })
        .await
    }

    /// 知识库总 chunk 数（§6 "5 万 chunk 起提示知识库偏大"，前端拿这个数字判断
    /// 要不要显示警告，不用把全部 chunk 都读回来才能数数）。
    pub async fn count_kb_chunks(&self) -> Result<u64> {
        self.call(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM kb_chunks", [], |r| r.get(0))
                .map_err(db_err)?;
            Ok(u64::try_from(n).unwrap_or(0))
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
        trust_level: parse_col(row, 10)?,
        server_hint: row.get(11)?,
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

fn row_to_scope(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncScope> {
    Ok(SyncScope {
        id: row.get(0)?,
        kind: parse_col(row, 1)?,
        local_path: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn row_to_sync_file_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncFileEntry> {
    Ok(SyncFileEntry {
        scope_id: row.get(0)?,
        rel_path: row.get(1)?,
        size: get_u64(row, 2)?,
        mtime: row.get(3)?,
        hash: row.get(4)?,
        present_local: row.get(5)?,
    })
}

fn row_to_remote_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteIndexEntry> {
    Ok(RemoteIndexEntry {
        device_id: row.get(0)?,
        rel_path: row.get(1)?,
        size: get_u64(row, 2)?,
        hash: row.get(3)?,
        seen_at: row.get(4)?,
    })
}

fn row_to_share(row: &rusqlite::Row<'_>) -> rusqlite::Result<Share> {
    Ok(Share {
        id: row.get(0)?,
        token: row.get(1)?,
        rel_path: row.get(2)?,
        permission: row.get(3)?,
        expires_at: row.get(4)?,
        status: row.get(5)?,
        created_at: row.get(6)?,
        link: String::new(),
    })
}

fn row_to_share_access(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShareAccess> {
    Ok(ShareAccess {
        id: row.get(0)?,
        share_id: row.get(1)?,
        peer_id: row.get(2)?,
        action: row.get(3)?,
        at: row.get(4)?,
    })
}

fn row_to_download(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadTask> {
    // 选项列是 JSON（迁移 011）；解析失败按"没有选项"处理而不是让整行读取失败——
    // 一条坏掉的选项 JSON 不该让用户的整个下载列表打不开（同 settings 里
    // `get_json` "非法/旧值静默退回默认"的既有取舍）。
    let options: Option<String> = row.get(9)?;
    let options = options.and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(DownloadTask {
        id: row.get(0)?,
        kind: parse_col(row, 1)?,
        url: row.get(2)?,
        save_path: row.get(3)?,
        status: parse_col(row, 4)?,
        total_bytes: get_u64(row, 5)?,
        downloaded_bytes: get_u64(row, 6)?,
        error: row.get(7)?,
        created_at: row.get(8)?,
        options,
    })
}

fn row_to_archive_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveRule> {
    let match_json: String = row.get(4)?;
    let action_json: String = row.get(5)?;
    Ok(ArchiveRule {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get(2)?,
        position: row.get(3)?,
        matcher: parse_json_col(4, &match_json)?,
        action: parse_json_col(5, &action_json)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_archive_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveEntry> {
    let model_meta_json: Option<String> = row.get(4)?;
    let model_meta = model_meta_json
        .map(|raw| parse_json_col::<ModelMeta>(4, &raw))
        .transpose()?;
    Ok(ArchiveEntry {
        id: row.get(0)?,
        current_path: row.get(1)?,
        category: parse_col(row, 2)?,
        size: get_u64(row, 3)?,
        model_meta,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn row_to_archive_tag(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveTag> {
    Ok(ArchiveTag {
        entry_id: row.get(0)?,
        tag: row.get(1)?,
        source: parse_col(row, 2)?,
    })
}

fn row_to_archive_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveLogEntry> {
    Ok(ArchiveLogEntry {
        id: row.get(0)?,
        entry_id: row.get(1)?,
        from_path: row.get(2)?,
        to_path: row.get(3)?,
        rule_id: row.get(4)?,
        at: row.get(5)?,
        undone: row.get(6)?,
    })
}

fn row_to_kb_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<KbSource> {
    Ok(KbSource {
        id: row.get(0)?,
        path: row.get(1)?,
        created_at: row.get(2)?,
    })
}

fn row_to_kb_document(row: &rusqlite::Row<'_>) -> rusqlite::Result<KbDocument> {
    Ok(KbDocument {
        id: row.get(0)?,
        source_id: row.get(1)?,
        rel_path: row.get(2)?,
        mtime: row.get(3)?,
        hash: row.get(4)?,
        status: parse_col(row, 5)?,
        updated_at: row.get(6)?,
    })
}

/// `kb_chunks.embedding` 的编码：f32 LE 逐个拼接（ARCHIVE_DESIGN §6），不引入
/// `byteorder` 依赖——`f32::to_le_bytes`/`from_le_bytes` 就是这四行代码。
fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// 长度不是 4 的倍数的坏 BLOB（不应该发生，只有这一处写入）忽略尾部余量，
/// 不 panic——同项目一贯"解析外部/持久化数据时容错优先"的惯例。
fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 解析存成 TEXT 的 JSON 列（`match_json`/`action_json`/`model_meta_json`）；解析失败
/// 视为列类型错误，同 `parse_col` 对非法枚举值的既有处理方式。
fn parse_json_col<T: serde::de::DeserializeOwned>(idx: usize, raw: &str) -> rusqlite::Result<T> {
    serde_json::from_str(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("invalid json: {e}").into(),
        )
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

fn get_u32(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<u32> {
    let v: i64 = row.get(idx)?;
    Ok(u32::try_from(v).unwrap_or(0))
}
