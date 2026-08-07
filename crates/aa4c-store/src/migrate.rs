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
    include_str!("migrations/009_archive.sql"),
    include_str!("migrations/010_knowledge.sql"),
    include_str!("migrations/011_download_options.sql"),
    include_str!("migrations/012_transfer_paused.sql"),
];

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_err)?;
    if usize::try_from(current).unwrap_or(0) >= MIGRATIONS.len() {
        // 已是最新：一条迁移都不跑，也就不去动 `foreign_keys` 这个连接级 PRAGMA
        // （`Store::open` 刚把它设成 ON，正常启动路径不该被迁移逻辑碰一下再设回来）。
        return Ok(());
    }

    // **迁移期间必须关掉外键强制**：SQLite 改不了已有的 CHECK 约束，唯一办法是
    // 「建新表 → 拷数据 → 删旧表 → 改名」（迁移 012 就是这么给 transfer_tasks.status
    // 加 'paused' 的）。而 `DROP TABLE` 在外键开启时会先做一次隐式 DELETE，顺着
    // `transfer_files.task_id ... ON DELETE CASCADE` 把子表数据**连带删光**——即整个
    // 传输历史的文件明细。这是 SQLite 官方 "Making Other Kinds Of Table Schema
    // Changes" 流程的第一步，且**必须在事务之外**设置：该 PRAGMA 在事务内是 no-op。
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(db_err)?;
    let result = run_pending(conn, current);
    // 不管成败都要还原——否则后续正常读写会在没有外键保护的连接上跑。
    let restored = conn
        .pragma_update(None, "foreign_keys", true)
        .map_err(db_err);
    result.and(restored)?;

    // 官方流程收尾：重建表期间外键是关着的，回来后校验一次没有留下悬空引用。
    let violations: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .map_err(db_err)?;
    if violations > 0 {
        return Err(Aa4cError::Db(format!(
            "database migration left {violations} dangling foreign key reference(s)"
        )));
    }
    Ok(())
}

/// 逐条跑还没应用的迁移，每条一个事务、失败整体回滚。
fn run_pending(conn: &mut Connection, current: i64) -> Result<()> {
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
