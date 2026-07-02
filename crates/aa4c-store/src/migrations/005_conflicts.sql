-- V0.2 里程碑 5：冲突记录（DATABASE_SCHEMA.md §4.5，SYNC_DESIGN.md §8）。
-- 同一限定基准路径 rel_path 存在多个不同 hash 的版本时，每个版本一行；
-- 一个 rel_path 有 ≥2 行即为一处冲突。由统一视图实时探测、整体替换（replace_conflicts）。
CREATE TABLE sync_conflicts (
    rel_path   TEXT NOT NULL,                  -- 限定基准路径（未加序号）
    hash       TEXT NOT NULL,                  -- 该版本的 BLAKE3 hex
    status     TEXT NOT NULL DEFAULT 'open'
               CHECK (status IN ('open','resolved')),
    created_at INTEGER NOT NULL,               -- 首次探测到该版本冲突的时间（unix 毫秒）
    PRIMARY KEY (rel_path, hash)
);

CREATE INDEX idx_sync_conflicts_path ON sync_conflicts(rel_path);
