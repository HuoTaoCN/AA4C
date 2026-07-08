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

> §4.1–§4.5 **全部已实现**（迁移 `002_trust.sql` + `003_sync.sql` +
> `004_remote_index.sql` + `005_conflicts.sql`，user_version=5）。V0.2 同步五个里程碑
> 完成。完整设计见 [SYNC_DESIGN.md](SYNC_DESIGN.md)。

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

### 4.4 remote_index —— 远端设备广播来的条目（已实现）

```sql
CREATE TABLE remote_index (
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    rel_path   TEXT NOT NULL,
    size       INTEGER NOT NULL,
    hash       TEXT,
    seen_at    INTEGER NOT NULL,                 -- 最近一次收到该条目的时间
    PRIMARY KEY (device_id, rel_path)
);

CREATE INDEX idx_remote_index_path ON remote_index(rel_path);
```

- 维护者：`aa4c-core` 的 `sync_exchange`（上线即与在线的完全信任设备交换索引摘要，
  整体 `Store::replace_remote_index` 单事务替换该设备的全部条目）；统一视图由
  `unified::merge`（本机 `sync_file_index` ⊕ `remote_index`）按限定 `rel_path` 归并。
- `rel_path` 已是限定展示路径（顶层段为来源分组「收到的」/共享文件夹名），与本机
  `sync_file_index` 同命名空间，便于按路径归并（SYNC_DESIGN.md §3.4）。
- 交换只走元数据（rel_path/size/hash），不传内容；按需拉取内容是里程碑 4。

> 黄/红判定：某 `rel_path` 本机 `present_local=0` 时，看持有它的 `device_id` 是否在线（`devices.last_seen_at` 30s 内）——在线为黄、仅离线为红。`remote_index` 可常驻内存 + 落库缓存。
> 解除配对（`DELETE devices`）经外键级联清空其 `remote_index`；**完全信任降为朋友**会保留设备行，应用层在 `Core::set_trust_level` 里**显式 `Store::clear_remote_index(device_id)`**（见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §2）。

### 4.5 sync_conflicts —— 冲突记录（已实现）

```sql
CREATE TABLE sync_conflicts (
    rel_path   TEXT NOT NULL,                  -- 限定基准路径（未加序号）
    hash       TEXT NOT NULL,                  -- 该版本的 BLAKE3 hex
    status     TEXT NOT NULL DEFAULT 'open'
               CHECK (status IN ('open','resolved')),
    created_at INTEGER NOT NULL,               -- 首次探测到该版本冲突的时间（unix 毫秒）
    PRIMARY KEY (rel_path, hash)
);

CREATE INDEX idx_sync_conflicts_path ON sync_conflicts(rel_path);
```

- 实现调整：设计初稿用 `local_hash`/`remote_hash` 建模一对本地-远端冲突；落地时改为
  **每个冲突版本一行**（`(rel_path, hash)` 联合主键），天然支持同一路径 ≥3 个版本的多方冲突。
  一个 `rel_path` 有 ≥2 行即一处冲突。
- 维护者：`aa4c-core` 的 `list_unified_files` 每次刷新时，把 `unified::merge` 探测到的当前冲突
  整体 `Store::replace_conflicts`（单事务 diff：删除已消失的、保留仍在版本的 `created_at`、
  插入新出现的）。冲突解决（用户拉取某版本、`.aa4c-part` 落盘自动加序号成为独立路径，或删掉多余
  副本）后，下次刷新该路径不再多版本，对应行随之清空。

## 4c. V0.3 表结构（远程连接 + 分享）

> 对应 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)（AA Connect）。§4c.0（`devices.server_hint`
> 列 + `settings` KV）**已实现**（迁移 `006_server_hint.sql`，user_version=6，里程碑 C2）；
> §4c.1/4c.2（`shares`/`share_access`）**已实现**（迁移 `007_shares.sql`，user_version=7，
> 里程碑 C6）。
> 连接配置复用现有 `settings` KV 表（不新增表）：**`server_url`**（自建 `aa4c-server` 地址，
> 格式 `aa4c://host:port#<证书指纹前16位hex>`，中继端点由服务器下发、不单独配置）、
> **`enable_remote`**（远程总开关，**默认 `false`**）。

### 4c.0 devices 增列 —— 对端 home server（CONNECT_DESIGN §3.4，已实现列，里程碑 C2）

```sql
ALTER TABLE devices ADD COLUMN server_hint TEXT;  -- 对端自建服务器地址（含指纹），可空
```

- **列已建、查询逻辑已接（`resolve_peer` 的 Lookup 兜底）；但配对协议尚未交换这个字段**——
  `PairRequest`/`PairAccept`/`DeviceInfo` 是既有 bincode 结构体变体，追加字段会破坏 v1/v2
  解码，需要一条新的追加消息才能安全传递，目前恒为 `NULL`（里程碑 C2 有意缩小的范围，见
  PROTOCOL.md §11、HANDOFF.md）。
- 现状：`resolve_peer` 的远程兜底只向**自己配置的服务器**查询（`aa4c-core::server_link`），
  覆盖「自己的多台设备共用同一服务器」这一主场景；跨服务器的好友寻址（真正用到
  `server_hint` 挑选对端服务器）留待 `server_hint` 的线路层交换实现后才生效。
- 纯局域网设备（对端未配置服务器）为 NULL，行为退化为 V0.2。

### 4c.1 shares —— 分享记录（CONNECT_DESIGN §7，已实现，里程碑 C6）

```sql
CREATE TABLE shares (
    id          TEXT PRIMARY KEY,                -- UUID
    token       TEXT NOT NULL UNIQUE,            -- ≥128bit 熵随机串（base58），即访问能力（capability）
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
```

- `token` 是能力：持有有效且未过期未吊销的 token 即可按 `permission` 访问，无需账号——包括
  **从未配对过的设备**（`Message::ShareRequest` 分发不检查 `trusted`，见 PROTOCOL.md §16）。
- 吊销 = 置 `status='revoked'`（保留审计），`Store::revoke_share` 目前不支持硬删除。
- 服务端每次访问校验：`status='open'` 且（`expires_at IS NULL` 或 `expires_at > now`）；不区分
  「不存在」/「过期」/「已吊销」，统一回 `Cancel{reason:"invalid_or_expired_token"}`
  （不泄露 token 存在性，同 Lookup 的既有防探测惯例）。
- **`transfer_tasks.peer_device_id` 的外键含义变化**（实现时发现的真实约束冲突）：该外键
  假设「peer 必然是已配对设备」，这个假设在 C6 之前对所有消息类型都成立（`trusted` 是
  Offer/FetchRequest/IndexRequest 的前提）；`ShareRequest` 打破了这个假设——服务/客户端两侧
  的 `serve_fetch`/`fetch::drive` 现在都先查一次对端是否已知（`store.get_device`），未知时
  跳过 `insert_task`/`update_task_status`（协议本身不受影响，只是这次传输不出现在「记录」页）。
  这不是 shares 表本身的字段，而是分享链接这个新能力对既有 `transfer_tasks` 表隐含假设的
  一处影响，记在这里便于以后排查。

### 4c.2 share_access —— 访问记录（可选，供「查看访问记录」，已实现，里程碑 C6）

```sql
CREATE TABLE share_access (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    share_id   TEXT NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
    peer_id    TEXT,                             -- 访问方 device_id；匿名访问为 NULL
    action     TEXT NOT NULL,                    -- 'list' / 'download' / 'upload'
    at         INTEGER NOT NULL                  -- unix 毫秒
);

CREATE INDEX idx_share_access_share ON share_access(share_id);
```

## 4e. V0.4 表结构（下载中心，设计定稿 v1，尚未建表）

> 对应 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) §4。首个里程碑 D1 只用到 `kind='http'`（Aria2）；
> `kind='bt'`（qBittorrent）留给 D2。

### 4e.1 download_tasks —— 下载任务

```sql
CREATE TABLE download_tasks (
    id                TEXT PRIMARY KEY,          -- aria2 GID，直接复用，不二次映射
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
```

- 没有 `peer_device_id` 之类的设备关联字段——下载任务天然没有"对端设备"，这也是不复用
  `transfer_tasks` 的直接原因（该表的 `peer_device_id` 有 `REFERENCES devices(id)` 外键，是
  "peer 必然是已配对设备"这个假设的产物，V0.4 的任务完全不适用这个假设，见 §4c.1 记录的
  C6 教训——不共用这张表就不会重蹈覆辙）。
- 速度/ETA 不落库，只在事件里带、前端本地维护（同 `transfer_tasks` 的既有先例）。

## 4f. 更远期预留（设计预告）

| 版本 | 表 | 用途 |
|------|----|------|
| V0.5 | `tags` / `file_tags` | AI 标签 |
| V0.5 | `archive_rules` | 归档规则（匹配条件 → 目标目录） |

> 若 V0.5 文件索引规模超出 SQLite 舒适区（千万级行），再评估迁移 RocksDB；元数据表保留在 SQLite。

## 5. 迁移策略

使用 `PRAGMA user_version` 做版本控制：

```rust
const MIGRATIONS: &[&str] = &[
    /* v1: */ include_str!("migrations/001_init.sql"),
    /* v2: */ include_str!("migrations/002_trust.sql"),        // devices.trust_level
    /* v3: */ include_str!("migrations/003_sync.sql"),         // sync_scopes + sync_file_index
    /* v4: */ include_str!("migrations/004_remote_index.sql"), // remote_index
    /* v5: */ include_str!("migrations/005_conflicts.sql"),    // sync_conflicts
    /* v6: */ include_str!("migrations/006_server_hint.sql"),  // devices.server_hint
    /* v7: */ include_str!("migrations/007_shares.sql"),       // shares + share_access
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
