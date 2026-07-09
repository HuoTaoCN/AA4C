//! 数据库迁移（DATABASE_SCHEMA.md §5）。
//!
//! 规则：迁移只追加、永不修改已发布的迁移；每个迁移一个事务，失败整体回滚。

use aa4c_types::{Aa4cError, Result};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/001_init.sql"),
    include_str!("migrations/002_trust.sql"),
    include_str!("migrations/003_sync.sql"),
    include_str!("migrations/004_remote_index.sql"),
    include_str!("migrations/005_conflicts.sql"),
    include_str!("migrations/006_server_hint.sql"),
    include_str!("migrations/007_shares.sql"),
    include_str!("migrations/008_downloads.sql"),
];

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_err)?;

    for (i, sql) in MIGRATIONS
        .iter()
        .enumerate()
        .skip(usize::try_from(current).unwrap_or(0))
    {
        let version = i64::try_from(i).unwrap_or(i64::MAX) + 1;
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute_batch(sql).map_err(db_err)?;
        tx.pragma_update(None, "user_version", version)
            .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        tracing::info!(version, "database migrated");
    }
    Ok(())
}

pub(crate) fn db_err(e: rusqlite::Error) -> Aa4cError {
    Aa4cError::Db(e.to_string())
}
