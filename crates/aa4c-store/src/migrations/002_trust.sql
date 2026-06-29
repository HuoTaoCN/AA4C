-- 002: 设备信任分级（DATABASE_SCHEMA.md §4 / SYNC_DESIGN.md §2）。
-- 旧的已配对设备（trusted=1）回填为 'friend'；用户可在设置升级为 'full'。

ALTER TABLE devices ADD COLUMN trust_level TEXT NOT NULL DEFAULT 'friend'
    CHECK (trust_level IN ('full', 'friend'));

CREATE INDEX idx_devices_trust_level ON devices(trust_level);
