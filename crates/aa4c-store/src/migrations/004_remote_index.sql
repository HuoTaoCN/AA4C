-- V0.2 里程碑 3：跨设备索引交换（DATABASE_SCHEMA.md §4.4，SYNC_DESIGN.md §3.3）。
-- 远端完全信任设备广播来的索引条目，配合设备在线判定决定统一视图的黄/红。
-- rel_path 已是限定展示路径（顶层段为来源分组），与本机 sync_file_index 同命名空间。
CREATE TABLE remote_index (
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    rel_path   TEXT NOT NULL,
    size       INTEGER NOT NULL,
    hash       TEXT,
    seen_at    INTEGER NOT NULL,             -- 最近一次收到该条目的时间（unix 毫秒）
    PRIMARY KEY (device_id, rel_path)
);

CREATE INDEX idx_remote_index_path ON remote_index(rel_path);
