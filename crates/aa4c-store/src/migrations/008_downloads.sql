-- V0.4 里程碑 D1：下载中心（DATABASE_SCHEMA.md §4e，DOWNLOAD_DESIGN.md §4）。
-- id 直接复用下载引擎原生任务号（aria2 GID），不做二次 UUID 映射。
CREATE TABLE download_tasks (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL DEFAULT 'http'
                      CHECK (kind IN ('http','bt')),   -- 'bt' 留给 D2（qBittorrent）
    url               TEXT NOT NULL,              -- 原始 URL / magnet URI
    save_path         TEXT,                       -- 落盘路径（完成后由引擎汇报回填）
    status            TEXT NOT NULL DEFAULT 'waiting'
                      CHECK (status IN ('active','waiting','paused','error','complete','removed')),
    total_bytes       INTEGER NOT NULL DEFAULT 0,
    downloaded_bytes  INTEGER NOT NULL DEFAULT 0,
    error             TEXT,                       -- 失败原因（人类可读）
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX idx_download_tasks_status ON download_tasks(status);
