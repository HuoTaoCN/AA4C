-- V0.3 里程碑 C2：对端 home server 地址（DATABASE_SCHEMA.md §4c.0，CONNECT_DESIGN.md §3.4）。
-- 格式 aa4c://host:port#指纹，可空——纯局域网设备（对端未配置服务器）行为退化为 V0.2。
-- 本迁移只建列；线路层交换（配对时/在线时把这个值同步给对端）留待后续里程碑，
-- 见 HANDOFF.md 的已知缺口说明。
ALTER TABLE devices ADD COLUMN server_hint TEXT;
