-- V0.3 里程碑 C6：分享链接（DATABASE_SCHEMA.md §4c.1/4c.2，CONNECT_DESIGN.md §7/§8）。
CREATE TABLE shares (
    id          TEXT PRIMARY KEY,                -- UUID
    token       TEXT NOT NULL UNIQUE,            -- >=128bit 熵随机串（base58），即访问能力（capability）
    rel_path    TEXT NOT NULL,                   -- 被分享的目标：限定路径，必须落在共享范围内（已索引），
                                                 -- 复用 V0.2 resolve_shared 解析与防穿越边界
    permission  TEXT NOT NULL DEFAULT 'read'
                CHECK (permission IN ('read','readwrite')),  -- V0.3 首版只用 read（readwrite 留余量）
    expires_at  INTEGER,                         -- 绝对过期时间（unix 毫秒）；NULL=长期
    status      TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','revoked')),
    created_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_shares_token ON shares(token);

CREATE TABLE share_access (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    share_id   TEXT NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
    peer_id    TEXT,                             -- 访问方 device_id；匿名访问为 NULL
    action     TEXT NOT NULL,                    -- 'list' / 'download' / 'upload'
    at         INTEGER NOT NULL                  -- unix 毫秒
);

CREATE INDEX idx_share_access_share ON share_access(share_id);
