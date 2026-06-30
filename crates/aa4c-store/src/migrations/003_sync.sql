-- 003: 同步范围 + 本机文件索引（DATABASE_SCHEMA.md §4.2-4.3 / SYNC_DESIGN.md §3、§6）。
-- V0.2 里程碑 2：只落本机索引；跨设备 remote_index 留给后续里程碑。

CREATE TABLE sync_scopes (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('folder','inbox')),
    local_path  TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'ondemand'
                CHECK (mode IN ('ondemand','mirror')),  -- V0.2 首版只用 ondemand
    created_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_sync_scopes_path ON sync_scopes(local_path);

CREATE TABLE sync_file_index (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_id      TEXT NOT NULL REFERENCES sync_scopes(id) ON DELETE CASCADE,
    rel_path      TEXT NOT NULL,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,
    hash          TEXT,
    present_local INTEGER NOT NULL DEFAULT 1,
    updated_at    INTEGER NOT NULL,
    UNIQUE (scope_id, rel_path)
);

CREATE INDEX idx_sync_file_index_scope ON sync_file_index(scope_id);
