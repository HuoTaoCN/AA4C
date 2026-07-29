-- V0.5 里程碑 AI1：归档（DATABASE_SCHEMA.md §4g，ARCHIVE_DESIGN.md §4）。
-- 预设规则随首次启动由应用代码写入（不是这份迁移），且默认停用（enabled=0）——
-- 迁移只建表结构，不塞业务数据，同既有惯例一致。
CREATE TABLE archive_rules (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL,
    match_json   TEXT NOT NULL,
    action_json  TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE TABLE archive_entries (
    id               TEXT PRIMARY KEY,
    current_path     TEXT NOT NULL,
    category         TEXT NOT NULL,
    size             INTEGER NOT NULL,
    model_meta_json  TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE archive_tags (
    entry_id  TEXT NOT NULL REFERENCES archive_entries(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    source    TEXT NOT NULL CHECK (source IN ('rule','ai','user')),
    PRIMARY KEY (entry_id, tag)
);

CREATE TABLE archive_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id   TEXT NOT NULL,
    from_path  TEXT NOT NULL,
    to_path    TEXT NOT NULL,
    rule_id    TEXT,
    at         INTEGER NOT NULL,
    undone     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_archive_log_entry ON archive_log(entry_id);
