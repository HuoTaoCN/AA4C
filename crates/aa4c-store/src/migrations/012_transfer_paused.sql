-- AA Send 暂停/继续：`transfer_tasks.status` 的 CHECK 约束加一个 'paused'
-- （PROTOCOL.md §7 / 打磨计划第二步）。
--
-- SQLite 改不了已有的 CHECK 约束，只能走官方的「建新表 → 拷数据 → 删旧表 → 改名」
-- 流程。`migrate.rs` 已经在事务之外把 `foreign_keys` 关掉了——不关的话下面的
-- DROP TABLE 会顺着 `transfer_files.task_id REFERENCES transfer_tasks(id)
-- ON DELETE CASCADE` 把整个传输历史的文件明细一起删掉。
--
-- 除 status 的取值集合外，表结构与 001_init.sql 完全一致，逐字段照抄（含外键与默认
-- 值）——重建表最容易出的错就是抄漏一个约束，这里刻意不做任何"顺手改进"。

CREATE TABLE transfer_tasks_new (
    id                 TEXT PRIMARY KEY,        -- UUID v4
    direction          TEXT NOT NULL CHECK (direction IN ('send','recv')),
    peer_device_id     TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    status             TEXT NOT NULL
                       CHECK (status IN ('waiting_accept','transferring','paused','done',
                                         'failed','cancelled','rejected')),
    total_bytes        INTEGER NOT NULL DEFAULT 0,
    transferred_bytes  INTEGER NOT NULL DEFAULT 0,
    file_count         INTEGER NOT NULL DEFAULT 0,
    save_dir           TEXT,                    -- 接收任务的保存目录（send 任务为 NULL）
    error              TEXT,                    -- 失败原因（人类可读）
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

INSERT INTO transfer_tasks_new
    (id, direction, peer_device_id, status, total_bytes, transferred_bytes,
     file_count, save_dir, error, created_at, updated_at)
SELECT
     id, direction, peer_device_id, status, total_bytes, transferred_bytes,
     file_count, save_dir, error, created_at, updated_at
FROM transfer_tasks;

DROP TABLE transfer_tasks;

ALTER TABLE transfer_tasks_new RENAME TO transfer_tasks;

-- 索引随旧表一起被 DROP 掉了，照 001_init.sql 原样重建。
CREATE INDEX idx_tasks_created ON transfer_tasks(created_at DESC);
CREATE INDEX idx_tasks_peer    ON transfer_tasks(peer_device_id);
CREATE INDEX idx_tasks_status  ON transfer_tasks(status);
