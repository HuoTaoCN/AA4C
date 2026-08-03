-- V0.5 里程碑 AI4：本地知识库（DATABASE_SCHEMA.md §4h，ARCHIVE_DESIGN.md §6）。
-- 独立于 009_archive.sql 单独一次迁移——AI1 与 AI4 之间隔着 AI2/AI3，先做的表
-- 不该等后做的表一起才落地。
CREATE TABLE kb_sources (
    id          TEXT PRIMARY KEY,
    path        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE kb_documents (
    id          TEXT PRIMARY KEY,
    source_id   TEXT NOT NULL REFERENCES kb_sources(id) ON DELETE CASCADE,
    rel_path    TEXT NOT NULL,
    mtime       INTEGER NOT NULL,
    hash        TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('pending','indexed','failed')),
    updated_at  INTEGER NOT NULL
);

CREATE TABLE kb_chunks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id     TEXT NOT NULL REFERENCES kb_documents(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    text       TEXT NOT NULL,
    embedding  BLOB NOT NULL,
    dims       INTEGER NOT NULL
);

CREATE INDEX idx_kb_documents_source ON kb_documents(source_id);
CREATE INDEX idx_kb_chunks_doc ON kb_chunks(doc_id);
