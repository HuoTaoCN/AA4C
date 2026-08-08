-- V0.7 里程碑 R2：信任传递 / 引荐（TRUST_DESIGN.md §5.6，DATABASE_SCHEMA.md §2b）。
--
-- 不新建表：被引荐但尚未确认的设备就是一条 trusted = 0 的 devices 记录，名字、平台、
-- public_key、server_hint 全部复用既有列。两列都是 ALTER TABLE ADD COLUMN，**不触发
-- 建表重建**——迁移 012 的教训（DROP TABLE 会顺着 ON DELETE CASCADE 删掉子表数据）
-- 见 DATABASE_SCHEMA.md §2.2。
--
-- 待确认列表 = introduced_by IS NOT NULL AND trusted = 0 AND introduce_dismissed = 0。
-- 确认后 trusted 置 1（自然移出列表），introduced_by 保留作为来源溯源。

ALTER TABLE devices ADD COLUMN introduced_by TEXT;                            -- 谁引荐的；NULL = 不是引荐来的
ALTER TABLE devices ADD COLUMN introduce_dismissed INTEGER NOT NULL DEFAULT 0; -- 用户点过「忽略」，不再打扰
