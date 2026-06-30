# AA4C Database Schema（V0.1）

> SQLite 数据库设计。与 [API_DESIGN.md](API_DESIGN.md) 中的 `aa4c-store` 接口对应。

## 1. 总体约定

| 项 | 约定 |
|----|------|
| 引擎 | SQLite（`rusqlite`，bundled 编译，无系统依赖） |
| 文件位置 | `<data_dir>/aa4c.db`（如 macOS：`~/Library/Application Support/aa4c/aa4c.db`） |
| 日志模式 | `PRAGMA journal_mode = WAL`（读写并发） |
| 外键 | `PRAGMA foreign_keys = ON` |
| 时间戳 | 一律 **unix 毫秒**（INTEGER），UTC |
| 主键 | 业务实体用 TEXT（DeviceId / UUID），明细行用 INTEGER 自增 |
| 枚举 | TEXT + CHECK 约束（可读性优先，量级很小） |
| 迁移 | `PRAGMA user_version`，启动时按版本号顺序执行（见 §5） |

数据库只存**元数据**，文件内容永远在文件系统中。

## 2. V0.1 表结构

### 2.1 devices —— 设备表

```sql
CREATE TABLE devices (
    id            TEXT PRIMARY KEY,             -- 设备指纹 = BLAKE3(公钥) hex
    name          TEXT NOT NULL,                -- 用户可见设备名
    platform      TEXT NOT NULL
                  CHECK (platform IN ('windows','macos','linux','android','ios','server')),
    public_key    BLOB NOT NULL,                -- Ed25519 公钥（32 字节）
    trusted       INTEGER NOT NULL DEFAULT 0,   -- 是否已配对（0/1）
    paired_at     INTEGER,                      -- 配对完成时间
    last_seen_at  INTEGER,                      -- 最近一次在线时间
    last_addr     TEXT,                         -- 最近一次发现的地址 "ip:port"
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX idx_devices_trusted ON devices(trusted);
```

说明：

- 未配对但发现过的设备**不入库**（只在内存中），配对成功才写入，避免表被局域网陌生设备污染
- 解除配对 = 直接 `DELETE`，而非 `trusted = 0`

### 2.2 transfer_tasks —— 传输任务表

```sql
CREATE TABLE transfer_tasks (
    id                 TEXT PRIMARY KEY,        -- UUID v4
    direction          TEXT NOT NULL CHECK (direction IN ('send','recv')),
    peer_device_id     TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    status             TEXT NOT NULL
                       CHECK (status IN ('waiting_accept','transferring','done',
                                         'failed','cancelled','rejected')),
    total_bytes        INTEGER NOT NULL DEFAULT 0,
    transferred_bytes  INTEGER NOT NULL DEFAULT 0,
    file_count         INTEGER NOT NULL DEFAULT 0,
    save_dir           TEXT,                    -- 接收任务的保存目录（send 任务为 NULL）
    error              TEXT,                    -- 失败原因（人类可读）
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE INDEX idx_tasks_created ON transfer_tasks(created_at DESC);
CREATE INDEX idx_tasks_peer    ON transfer_tasks(peer_device_id);
CREATE INDEX idx_tasks_status  ON transfer_tasks(status);
```

说明：

- 进度（`transferred_bytes`）**节流写库**（≥1s 一次），实时进度走事件总线，不依赖数据库
- 应用启动时将所有 `waiting_accept` / `transferring` 状态的任务标记为 `failed`（error = "应用重启中断"）——V0.1 不做断点续传恢复

### 2.3 transfer_files —— 传输文件明细表

```sql
CREATE TABLE transfer_files (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT NOT NULL REFERENCES transfer_tasks(id) ON DELETE CASCADE,
    file_index  INTEGER NOT NULL,               -- 任务内序号（与协议 file_index 对应）
    rel_path    TEXT NOT NULL,                  -- 相对路径，'/' 分隔
    size        INTEGER NOT NULL,
    hash        TEXT,                           -- BLAKE3 hex，完成后填充
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending','transferring','done','failed')),
    abs_path    TEXT,                           -- 本地绝对路径（发送=源路径，接收=落盘路径）
    UNIQUE (task_id, file_index)
);

CREATE INDEX idx_files_task ON transfer_files(task_id);
```

### 2.4 settings —— 设置表（KV）

```sql
CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,                   -- JSON 编码的值
    updated_at INTEGER NOT NULL
);
```

V0.1 已定义的 key：

| key | 默认值 | 说明 |
|-----|--------|------|
| `device_name` | hostname | 本机设备名 |
| `save_dir` | `~/Downloads/AA4C` | 默认接收目录 |
| `auto_accept_from_trusted` | `false` | 已配对设备来文件是否免确认 |
| `listen_port` | `42420` | 监听端口 |

> 设备私钥**不存数据库**，单独存放于 `<data_dir>/identity/`，文件权限 0600。

## 3. 实体关系

```
devices 1 ──── n transfer_tasks 1 ──── n transfer_files
                      settings（独立 KV）
```

## 4. V0.2 表结构（信任分级 + 跨设备索引设计）

> §4.1 `devices.trust_level`、§4.2 `sync_scopes`、§4.3 `sync_file_index` **已实现**
> （迁移 `002_trust.sql` + `003_sync.sql`，user_version=3）；§4.4 `remote_index`、
> §4.5 `sync_conflicts` 仍为设计定稿、尚未建表，随后续里程碑落地。
> 完整设计见 [SYNC_DESIGN.md](SYNC_DESIGN.md)。

### 4.1 devices 增列 —— 信任分级（已实现）

```sql
ALTER TABLE devices ADD COLUMN trust_level TEXT NOT NULL DEFAULT 'friend'
    CHECK (trust_level IN ('full','friend'));   -- 临时/陌生不入库
```

- 旧数据回填：`trusted=1` 的行 → `trust_level='friend'`，用户可在设置升级为 `full`。
- 仅 `full`（完全信任）设备参与跨设备索引与同步。

### 4.2 sync_scopes —— 共享范围（已实现）

```sql
CREATE TABLE sync_scopes (
    id          TEXT PRIMARY KEY,                -- UUID
    kind        TEXT NOT NULL CHECK (kind IN ('folder','inbox')),
    local_path  TEXT NOT NULL,                   -- 本机绝对路径
    mode        TEXT NOT NULL DEFAULT 'ondemand'
                CHECK (mode IN ('ondemand','mirror')),  -- V0.2 首版只用 ondemand
    created_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_sync_scopes_path ON sync_scopes(local_path);
```

- `local_path` 唯一：同一文件夹不会被重复添加为两个范围。
- Inbox 是全局唯一一行（`kind='inbox'`），由 `Store::ensure_inbox_scope` 维护；
  `save_dir` 设置变化时原地更新 `local_path`，旧路径下的条目随下次扫描清空。

### 4.3 sync_file_index —— 本机文件索引（已实现）

```sql
CREATE TABLE sync_file_index (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_id      TEXT NOT NULL REFERENCES sync_scopes(id) ON DELETE CASCADE,
    rel_path      TEXT NOT NULL,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,              -- unix 毫秒
    hash          TEXT,                          -- BLAKE3 hex
    present_local INTEGER NOT NULL DEFAULT 1,    -- 内容是否在本机磁盘（绿）
    updated_at    INTEGER NOT NULL,
    UNIQUE (scope_id, rel_path)
);
```

- 维护者：`aa4c-core` 的 `sync_index::scan_scope`（遍历范围目录，mtime+size 未变复用旧
  hash，否则重新算 BLAKE3），结果整体写入 `Store::replace_scope_index`（单事务 diff：
  删除已消失的条目、插入/更新现存条目）。
- V0.2 里程碑 2 范围内 `present_local` 恒为 `1`（只有本机扫描出的条目）；跨设备的
  `present_local=0` 条目要等 §4.4 `remote_index` 落地才会出现于统一视图。
- 触发时机：启动时扫一次、每 300s 定时全量重扫、任意一次传输完成后追加一次重扫
  （`aa4c-core/src/sync_index.rs` 的 `spawn_background_scan`）。文件系统实时监听
  （`notify` crate）留给后续里程碑，见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §11。

### 4.4 remote_index —— 远端设备广播来的条目

```sql
CREATE TABLE remote_index (
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    rel_path   TEXT NOT NULL,
    size       INTEGER NOT NULL,
    hash       TEXT,
    seen_at    INTEGER NOT NULL,                 -- 最近一次收到该条目的时间
    PRIMARY KEY (device_id, rel_path)
);
```

> 黄/红判定：某 `rel_path` 本机 `present_local=0` 时，看持有它的 `device_id` 是否在线（`devices.last_seen_at` 30s 内）——在线为黄、仅离线为红。`remote_index` 可常驻内存 + 落库缓存。
> 解除配对（`DELETE devices`）经外键级联清空其 `remote_index`；**完全信任降为朋友**会保留设备行，需在应用层**显式 `DELETE FROM remote_index WHERE device_id=?`**（见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §2）。

### 4.5 sync_conflicts —— 冲突记录（占位）

```sql
CREATE TABLE sync_conflicts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    rel_path    TEXT NOT NULL,
    local_hash  TEXT,
    remote_hash TEXT,
    status      TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','resolved')),
    created_at  INTEGER NOT NULL
);
```

## 4b. 更远期预留（设计预告）

| 版本 | 表 | 用途 |
|------|----|------|
| V0.3 | `shares` | 分享链接（token、权限、过期时间、访问记录） |
| V0.4 | `download_tasks` | 下载任务（url、协议、引擎、状态） |
| V0.5 | `tags` / `file_tags` | AI 标签 |
| V0.5 | `archive_rules` | 归档规则（匹配条件 → 目标目录） |

> 若 V0.5 文件索引规模超出 SQLite 舒适区（千万级行），再评估迁移 RocksDB；元数据表保留在 SQLite。

## 5. 迁移策略

使用 `PRAGMA user_version` 做版本控制：

```rust
const MIGRATIONS: &[&str] = &[
    /* v1: */ include_str!("migrations/001_init.sql"),
    // v2: 002_trust_and_sync.sql（V0.2：devices.trust_level + sync_scopes
    //      + sync_file_index + remote_index + sync_conflicts，见 §4）
];

fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;       // 每个迁移一个事务
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
```

规则：

1. 迁移文件**只追加，永不修改**已发布的迁移
2. 每个迁移在独立事务中执行，失败则整体回滚、应用拒绝启动
3. 不做降级迁移；升级前数据库文件自动备份为 `aa4c.db.bak.<version>`

## 6. 性能注意事项

- 打开连接后执行：`PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;`
- `rusqlite` 连接不跨线程共享：用单一专职线程 + mpsc channel 包装成 async 接口（见 API_DESIGN §7）
- 批量插入 `transfer_files` 使用单事务 + prepared statement
