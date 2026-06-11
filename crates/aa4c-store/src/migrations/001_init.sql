-- V0.1 初始表结构（DATABASE_SCHEMA.md §2）

CREATE TABLE devices (
    id            TEXT PRIMARY KEY,             -- 设备指纹 = BLAKE3(公钥) hex
    name          TEXT NOT NULL,                -- 用户可见设备名
    platform      TEXT NOT NULL
                  CHECK (platform IN ('windows','macos','linux','android','ios','server')),
    public_key    BLOB NOT NULL,                -- Ed25519 公钥（32 字节）
    trusted       INTEGER NOT NULL DEFAULT 0,   -- 是否已配对（0/1）
    paired_at     INTEGER,                      -- 配对完成时间
    last_seen_at  INTEGER,                      -- 最近一次在线时间
    last_addr     TEXT,                         -- 最近一次发现的地址 "ip:port"
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX idx_devices_trusted ON devices(trusted);

CREATE TABLE transfer_tasks (
    id                 TEXT PRIMARY KEY,        -- UUID v4
    direction          TEXT NOT NULL CHECK (direction IN ('send','recv')),
    peer_device_id     TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    status             TEXT NOT NULL
                       CHECK (status IN ('waiting_accept','transferring','done',
                                         'failed','cancelled','rejected')),
    total_bytes        INTEGER NOT NULL DEFAULT 0,
    transferred_bytes  INTEGER NOT NULL DEFAULT 0,
    file_count         INTEGER NOT NULL DEFAULT 0,
    save_dir           TEXT,                    -- 接收任务的保存目录（send 任务为 NULL）
    error              TEXT,                    -- 失败原因（人类可读）
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE INDEX idx_tasks_created ON transfer_tasks(created_at DESC);
CREATE INDEX idx_tasks_peer    ON transfer_tasks(peer_device_id);
CREATE INDEX idx_tasks_status  ON transfer_tasks(status);

CREATE TABLE transfer_files (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT NOT NULL REFERENCES transfer_tasks(id) ON DELETE CASCADE,
    file_index  INTEGER NOT NULL,               -- 任务内序号（与协议 file_index 对应）
    rel_path    TEXT NOT NULL,                  -- 相对路径，'/' 分隔
    size        INTEGER NOT NULL,
    hash        TEXT,                           -- BLAKE3 hex，完成后填充
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending','transferring','done','failed')),
    abs_path    TEXT,                           -- 本地绝对路径（发送=源路径，接收=落盘路径）
    UNIQUE (task_id, file_index)
);

CREATE INDEX idx_files_task ON transfer_files(task_id);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,                   -- JSON 编码的值
    updated_at INTEGER NOT NULL
);
