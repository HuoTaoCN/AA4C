# AA4C API Design（V0.1）

> 本文档定义 V0.1 阶段所有 Rust 模块的公共接口、传输协议和 Tauri 前后端契约。
> **这是接口的唯一事实来源**：实现必须与本文档一致，接口变更必须先更新本文档。

## 1. 设计原则

1. **Core 只协调**：业务逻辑在各 Service crate 中，`aa4c-core` 负责组装与事件分发
2. **单向依赖**：`aa4c-types` 被所有 crate 依赖，crate 之间不互相依赖
3. **事件驱动**：服务向事件总线发布事件，UI 通过 Tauri 事件订阅
4. **异步优先**：所有 I/O 接口为 `async fn`（tokio runtime）
5. **错误统一**：所有公共接口返回 `Result<T, Aa4cError>`

## 2. Workspace 结构

```
AA4C/
├── Cargo.toml                 # workspace
├── crates/
│   ├── aa4c-types/            # 公共类型、错误、事件（无 I/O，依赖最少）
│   ├── aa4c-proto/            # 线路协议：Message 定义 + 帧编解码（配对与传输共用）
│   ├── aa4c-identity/         # 设备身份、密钥、配对协议
│   ├── aa4c-discovery/        # mDNS 设备发现
│   ├── aa4c-transfer/         # 文件传输引擎（收发）
│   ├── aa4c-store/            # SQLite 持久化
│   └── aa4c-core/             # Core：组装、生命周期、事件总线
└── apps/
    └── desktop/               # Tauri 2 + Vue3
        ├── src-tauri/         # Tauri 后端（依赖 aa4c-core）
        └── src/               # Vue3 前端
```

依赖关系（只允许向下依赖）：

```
aa4c-core → identity / discovery / transfer / store → aa4c-types
identity / transfer → aa4c-proto → aa4c-types
identity → aa4c-store（配对成功写库）
apps/desktop/src-tauri → aa4c-core
```

## 3. aa4c-types —— 公共类型

```rust
/// 设备 ID = 设备公钥的 BLAKE3 哈希（hex，64 字符）
pub type DeviceId = String;
/// 任务 ID = UUID v4 字符串
pub type TaskId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,            // 用户可见设备名，如 "Huo 的 MacBook"
    pub platform: Platform,
    pub version: String,         // AA4C 版本号
    pub addr: Option<SocketAddr>,// 最近一次发现的地址
    pub online: bool,
    pub trusted: bool,           // 是否已配对
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform { Windows, Macos, Linux, Android, Ios, Server }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTask {
    pub id: TaskId,
    pub direction: Direction,        // Send | Recv
    pub peer: DeviceId,
    pub files: Vec<TransferFile>,
    pub status: TransferStatus,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub created_at: i64,             // unix ms
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFile {
    pub rel_path: String,            // 相对路径（文件夹传输时保留层级）
    pub size: u64,
    pub hash: Option<String>,        // BLAKE3 hex，传输完成后填充
    pub status: FileStatus,          // Pending | Transferring | Done | Failed
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    WaitingAccept,   // 等待接收方确认
    Transferring,
    Done,
    Failed,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Send, Recv }
```

### 3.1 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum Aa4cError {
    #[error("device not found: {0}")]
    DeviceNotFound(DeviceId),
    #[error("device not paired: {0}")]
    NotPaired(DeviceId),
    #[error("pairing rejected")]
    PairingRejected,
    #[error("pairing pin mismatch")]
    PinMismatch,
    #[error("transfer rejected by peer")]
    TransferRejected,
    #[error("hash mismatch for {path}")]
    HashMismatch { path: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Aa4cError>;
```

### 3.2 事件（事件总线 + Tauri 事件共用）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CoreEvent {
    DeviceFound(DeviceInfo),
    DeviceLost { id: DeviceId },
    DeviceUpdated(DeviceInfo),

    /// 对方请求与本机配对
    PairingRequest { session_id: String, peer: DeviceInfo },
    /// 双方界面需要展示的 6 位确认码
    PairingPin { session_id: String, pin: String },
    PairingResult { session_id: String, peer: DeviceId, success: bool },

    /// 对方请求向本机发送文件
    TransferRequest { task: TransferTask },
    /// 出站连接已建立（里程碑 C4 连接质量）：只有发起方（发送/拉取）收得到，
    /// 只存当次会话内存，不落库。
    TransferConnected { task_id: TaskId, via: ConnectionVia },
    TransferProgress {
        task_id: TaskId,
        transferred_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
        current_file: String,
    },
    TransferDone { task_id: TaskId },
    TransferFailed { task_id: TaskId, error: String },

    /// 本机同步索引发生变化，UI 应重新拉取统一文件视图（里程碑 2）。
    SyncIndexUpdated,

    /// 下载进度（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md §5）：状态迁移必发，进行中按数秒级节流。
    DownloadProgress {
        task_id: TaskId,
        downloaded_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
    },
    DownloadDone { task_id: TaskId, save_path: String },
    DownloadFailed { task_id: TaskId, error: String },
}

/// 一次连接实际走的档位（CONNECT_DESIGN.md §2 连接阶梯，里程碑 C4 + C5）。
/// `Punch`（打洞后升级成的直连）在前端 UI 上并入「直连」显示，不单独暴露成第三个词。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionVia {
    Direct,
    Punch,
    Relay,
}
```

## 4. aa4c-identity —— 设备身份与配对

```rust
/// 本机身份：Ed25519 密钥对 + 自签名 TLS 证书。
/// 私钥存 `<data_dir>/identity/device.key`（PEM，0600）；
/// 证书每次启动由私钥重新自签（指纹固定在公钥上，证书本身可变）。
pub struct Identity { /* 字段私有 */ }

impl Identity {
    /// 加载或首次生成身份
    pub fn load_or_generate(data_dir: &Path) -> Result<Self>;
    pub fn device_id(&self) -> &DeviceId;        // = BLAKE3(public_key) hex
    pub fn public_key(&self) -> &[u8];           // Ed25519 公钥 32 字节

    /// mTLS：双方互验证书。expect_peer = Some(id) 时握手内强制指纹一致；
    /// None 时（监听端常规路径 / 首次配对）接受任意有效 Ed25519 证书，
    /// 由上层在握手后用 device_id_from_cert 校验 trusted 或走 PIN 确认
    pub fn tls_server_config(&self, expect_peer: Option<&DeviceId>) -> Result<rustls::ServerConfig>;
    pub fn tls_client_config(&self, expect_peer: Option<&DeviceId>) -> Result<rustls::ClientConfig>;
}

/// 从对端证书 DER 提取 DeviceId（仅接受 Ed25519 证书）
pub fn device_id_from_cert(cert: &CertificateDer<'_>) -> Result<DeviceId>;
/// 由公钥计算 DeviceId
pub fn device_id_from_public_key(public_key: &[u8]) -> DeviceId;
/// 配对 PIN（PROTOCOL.md §6.1）
pub fn derive_pin(pk_a: &[u8], pk_b: &[u8]) -> String;

/// 配对协议（发起方 = A，接收方 = B）：
/// 1. A 通过 TLS 连接 B，发送 PairRequest{A 的 DeviceInfo + 公钥}
/// 2. B 弹窗确认是否接受配对
/// 3. 双方各自计算 PIN = BLAKE3(min(pkA,pkB) || max(pkA,pkB))[0..3] % 1_000_000（6 位数字）
/// 4. 双方界面显示 PIN，用户目视核对一致后各自点击确认
/// 5. 双向确认后互发 PairConfirm，写入 devices 表（trusted = true）
pub struct PairingManager { /* ... */ }

impl PairingManager {
    pub fn new(identity: Arc<Identity>, store: Arc<Store>, events: EventSender) -> Self;
    /// 发起配对，返回 session_id；后续通过事件推进
    pub async fn start_pairing(&self, peer: &DeviceInfo) -> Result<String>;
    /// 本端用户确认/拒绝（配对请求方与接收方都要调用）
    pub async fn confirm(&self, session_id: &str, accept: bool) -> Result<()>;
}
```

## 5. aa4c-discovery —— 设备发现

```rust
/// mDNS 服务类型与 TXT 记录
pub const SERVICE_TYPE: &str = "_aa4c._tcp.local.";
/// TXT: id=<device_id>, name=<device_name>, platform=<platform>, ver=<version>, proto=1

pub struct DiscoveryService { /* mdns-sd ServiceDaemon */ }

impl DiscoveryService {
    pub fn new(self_info: DeviceInfo, events: EventSender) -> Result<Self>;
    /// 注册本机服务（广播）并开始浏览同类服务
    /// 发现/丢失设备时发布 DeviceFound / DeviceLost 事件
    pub async fn start(&self, listen_port: u16) -> Result<()>;
    pub async fn stop(&self) -> Result<()>;
    /// 当前发现的设备快照
    pub fn devices(&self) -> Vec<DeviceInfo>;
}
```

实现说明：

- 使用 `mdns-sd` crate；过滤掉自身（id 相同）
- 设备 30 秒无响应视为离线，发布 `DeviceLost`
- 监听端口默认 **42420**，被占用时自动递增并在 TXT 中广播实际端口

## 6. aa4c-transfer —— 传输引擎

### 6.1 公共接口

```rust
pub struct TransferService { /* ... */ }

impl TransferService {
    pub fn new(
        identity: Arc<Identity>,
        store: Store,            // Store 内部已是廉价克隆的句柄
        events: EventSender,
        config: TransferConfig,
    ) -> Arc<Self>;

    /// 启动 TLS 监听（接收端），返回实际监听端口
    pub async fn start_listener(&self, port: u16) -> Result<u16>;

    /// 发送文件/文件夹，立即返回 task_id，进度通过事件推送
    pub async fn send(&self, peer: &DeviceInfo, paths: Vec<PathBuf>) -> Result<TaskId>;

    /// 接收端：用户确认是否接收（save_dir 为空则使用默认下载目录）
    pub async fn accept(&self, task_id: &TaskId, accept: bool, save_dir: Option<PathBuf>) -> Result<()>;

    /// 取消任务（双方均可）
    pub async fn cancel(&self, task_id: &TaskId) -> Result<()>;
}

pub struct TransferConfig {
    pub chunk_size: usize,        // 默认 4 MiB
    pub default_save_dir: PathBuf,// 默认 ~/Downloads/AA4C（由 Core 注入平台目录）
    pub max_concurrent_tasks: usize, // 默认 4（发送端信号量）
    pub timeout: Duration,        // 协议等待超时，默认 60s（PROTOCOL §8）
}
```

### 6.2 线路协议（AA Transfer Protocol v1）

TLS 1.3 之上的消息流。消息帧格式：`[4 字节大端长度][bincode 编码的 Message]`；文件数据帧之后直接跟原始字节。

> `Message` 与帧编解码实现位于 `aa4c-proto`（配对与传输共用），提供
> `encode_frame / read_message / write_message`（强制 16 MiB 帧长上限）。

```rust
#[derive(Serialize, Deserialize)]
pub enum Message {
    /// 握手：声明协议版本与自身 DeviceId（与 TLS 证书指纹必须一致）
    Hello { proto: u16, device_id: DeviceId },
    HelloAck { proto: u16, device_id: DeviceId },

    /// 发送方 → 接收方：传输请求（文件清单）
    Offer { task_id: TaskId, files: Vec<FileMeta> },
    /// 接收方 → 发送方：接受 / 拒绝
    OfferAnswer { task_id: TaskId, accept: bool },

    /// 文件分块头，随后紧跟 len 字节的原始数据
    Chunk { file_index: u32, offset: u64, len: u32 },
    /// 单个文件结束，hash = 整文件 BLAKE3
    FileDone { file_index: u32, hash: String },
    /// 接收方校验结果
    FileAck { file_index: u32, ok: bool },

    TaskDone { task_id: TaskId },
    Cancel { task_id: TaskId, reason: String },
}

#[derive(Serialize, Deserialize)]
pub struct FileMeta {
    pub rel_path: String,   // 使用 '/' 分隔，接收端负责转换并防御路径穿越（拒绝 ".."、绝对路径）
    pub size: u64,
}
```

协议规则：

1. 握手后双方校验 `device_id` 是否在本地 `devices` 表中且 `trusted = true`，否则断开（`NotPaired`）
2. 接收端落盘到 `<save_dir>/<rel_path>.aa4c-part`，`FileAck(ok)` 后重命名为正式文件
3. 哈希校验失败 → `FileAck(ok=false)`，发送方重传该文件（最多 2 次），仍失败则任务 `Failed`
4. 进度事件按 ≥100ms 节流发布

## 7. aa4c-store —— 持久化

```rust
pub struct Store { /* rusqlite::Connection（专用线程 + channel 包装为 async） */ }

impl Store {
    /// 打开数据库并自动执行迁移（PRAGMA user_version）
    pub async fn open(db_path: &Path) -> Result<Self>;

    // 设备
    pub async fn upsert_device(&self, d: &DeviceRecord) -> Result<()>;
    pub async fn get_device(&self, id: &DeviceId) -> Result<Option<DeviceRecord>>;
    pub async fn list_paired_devices(&self) -> Result<Vec<DeviceRecord>>;
    pub async fn remove_device(&self, id: &DeviceId) -> Result<()>;

    // 传输任务
    pub async fn insert_task(&self, t: &TransferTask) -> Result<()>;
    pub async fn update_task_status(&self, id: &TaskId, status: TransferStatus, error: Option<&str>) -> Result<()>;
    pub async fn update_task_progress(&self, id: &TaskId, transferred: u64) -> Result<()>;
    pub async fn list_tasks(&self, limit: u32, offset: u32) -> Result<Vec<TransferTask>>;

    // 设置
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>>;
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()>;
}
```

表结构定义见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)。

## 8. aa4c-core —— 组装与事件总线

```rust
/// 事件总线 = tokio::sync::broadcast
pub type EventSender = tokio::sync::broadcast::Sender<CoreEvent>;

pub struct Core {
    pub identity: Arc<Identity>,
    pub store: Store,            // Store 自身已是廉价克隆句柄，无需再包 Arc
    pub discovery: Arc<DiscoveryService>,
    pub transfer: Arc<TransferService>,
    pub pairing: Arc<PairingManager>,
    events: EventSender,
    self_info: DeviceInfo,
    listen_port: u16,
}

impl Core {
    /// 完整启动序列：身份 → 数据库 → 遗留任务清理 → 配对/传输装配
    /// → 传输监听 → mDNS 广播
    pub async fn start(config: CoreConfig) -> Result<Arc<Core>>;
    pub async fn shutdown(&self) -> Result<()>;
    /// UI 订阅事件
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CoreEvent>;
    pub fn self_info(&self) -> DeviceInfo;
    pub fn listen_port(&self) -> u16;

    // §9 的 11 个 Command 在 Core 上有一一对应的编排方法，Tauri 层只做转发：
    // list_devices / start_pairing / confirm_pairing / unpair_device /
    // send_files / accept_transfer / cancel_transfer / list_transfers /
    // get_settings / update_settings
}

// 入站连接分流（M6）：传输与配对共用同一监听端口。传输监听器读到首条消息后，
// `Offer` 走接收会话，`PairRequest` 经 `aa4c-transfer::IncomingPairDispatch`
// 钩子转交 `PairingManager`；该钩子由 Core 在装配阶段注入（传输层不感知配对）。

pub struct CoreConfig {
    pub data_dir: PathBuf,      // 默认：dirs::data_dir()/aa4c
    pub device_name: Option<String>, // 默认：hostname
    pub listen_port: u16,       // 默认 42420
    pub transfer: TransferConfig,
}
```

## 9. Tauri 前后端契约

> 本契约同时服务**桌面端与 Android**（同一 Tauri 工程、同一 Vue3 前端）。
> Android 特有的原生能力（多播锁、MediaStore 导出）由 Kotlin 插件实现，不改变本节的 Command / Event 接口。

### 9.1 Commands（前端 `invoke`）

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `get_self_device` | — | `DeviceInfo` | 本机信息 |
| `list_devices` | — | `DeviceInfo[]` | 已发现 + 已配对设备（合并去重） |
| `start_pairing` | `deviceId` | `sessionId: string` | 向某设备发起配对 |
| `confirm_pairing` | `sessionId, accept: bool` | `void` | 确认 / 拒绝配对（PIN 核对后） |
| `unpair_device` | `deviceId` | `void` | 解除配对 |
| `send_files` | `deviceId, paths: string[]` | `taskId: string` | 发起 AA 发送 |
| `accept_transfer` | `taskId, accept: bool, saveDir?: string` | `void` | 接收端确认 |
| `cancel_transfer` | `taskId` | `void` | 取消任务 |
| `list_transfers` | `limit, offset` | `TransferTask[]` | 传输记录 |
| `get_settings` | — | `Settings` | 读取设置 |
| `update_settings` | `Settings` | `void` | 保存设置（设备名变更需重新广播 mDNS） |
| `create_share` | `relPath, expiresAt?: number` | `Share` | 生成分享链接（里程碑 C6，`relPath` 须落在共享范围内） |
| `list_shares` | — | `Share[]` | 列出本机全部分享（含完整链接） |
| `revoke_share` | `id` | `void` | 吊销一条分享 |
| `list_share_access` | `shareId` | `ShareAccess[]` | 某条分享的访问记录 |
| `open_share` | `link: string` | `taskId: string` | 打开一个分享链接，立即返回接收任务 id |
| `add_download` | `url: string` | `taskId: string` | 新建下载任务（里程碑 D1，`url` 为 HTTP/HTTPS/FTP 直链） |
| `pause_download` | `taskId` | `void` | 暂停一个下载任务 |
| `resume_download` | `taskId` | `void` | 继续一个已暂停的下载任务 |
| `cancel_download` | `taskId` | `void` | 取消（并从引擎移除）一个下载任务 |
| `list_downloads` | — | `DownloadTask[]` | 列出全部下载任务（按创建时间倒序） |

下载能力未就绪（aria2c 未打包/未启动成功，见 DOWNLOAD_DESIGN.md §3.1 健康检查降级）时，
上述 5 个下载 Command 一律返回 `{ code: "unavailable", message }`。

所有 Command 失败时返回 `{ code: string, message: string }`，`code` 取 `Aa4cError` 的变体名（如 `not_paired`）。

> 本表未逐一列出 V0.2/V0.3 陆续新增的全部 Command（如 `set_trust_level`、同步/统一视图相关的
> 几个 Command），也未追加 V0.4（下载中心批量操作等）、V0.5（归档/AI，见 [ARCHIVE_DESIGN.md](ARCHIVE_DESIGN.md)）
> 与 V0.7 R2（信任引荐：`list_pending_introductions` / `confirm_introduction` /
> `dismiss_introduction` / `refresh_introductions`，见 [TRUST_DESIGN.md](TRUST_DESIGN.md) §5）
> 新增的 Command——它们与上面列出的同构（Core 方法 1:1 映射），签名以 `apps/desktop/src/lib/api.ts`
> 为准；本表只保证覆盖 V0.1 基线 + 里程碑 C6（分享链接）新增部分。

### 9.2 Events（前端 `listen`）

Tauri 后端订阅 `Core::subscribe()`，将每个 `CoreEvent` 转发为 Tauri 事件，事件名 = `aa4c://` + 事件类型蛇形命名：

```
aa4c://device_found        payload: DeviceInfo
aa4c://device_lost         payload: { id }
aa4c://pairing_request     payload: { sessionId, peer }
aa4c://pairing_pin         payload: { sessionId, pin }
aa4c://pairing_result      payload: { sessionId, peer, success }
aa4c://transfer_request    payload: { task }
aa4c://transfer_connected  payload: { taskId, via }         // via: "direct" | "relay"（里程碑 C4）
aa4c://transfer_progress   payload: { taskId, transferredBytes, totalBytes, speedBps, currentFile }
aa4c://transfer_done       payload: { taskId }
aa4c://transfer_failed     payload: { taskId, error }
aa4c://sync_index_updated  payload: null
aa4c://introductions_updated payload: null                   // 收到新的设备引荐（里程碑 R2），UI 重拉待确认列表
aa4c://download_progress   payload: { taskId, downloadedBytes, totalBytes, speedBps }   // 里程碑 D1
aa4c://download_done       payload: { taskId, savePath }
aa4c://download_failed     payload: { taskId, error }
```

JSON 一律使用 **camelCase**（serde `rename_all = "camelCase"` 在 Tauri 层统一处理）。

## 10. 关键常量汇总

| 常量 | 值 |
|------|-----|
| mDNS 服务类型 | `_aa4c._tcp.local.` |
| 默认监听端口 | 42420 |
| 协议版本 | 1 |
| 分块大小 | 4 MiB |
| 哈希算法 | BLAKE3 |
| 配对 PIN | 6 位数字 |
| 设备离线判定 | 30 秒 |
| 临时文件后缀 | `.aa4c-part` |

## 11. Android 适配（V0.1 实验版）

与桌面端的差异点，全部收敛在平台层，core crate 不感知 Android：

| 关注点 | 方案 |
|--------|------|
| mDNS 多播 | Kotlin 插件在 `onCreate` 获取 `WifiManager.MulticastLock`，`onDestroy` 释放；Rust 侧无改动 |
| 数据目录 | `dirs` 在 Android 上不可用，改用 Tauri `app_handle.path()`（指向应用私有目录）；`CoreConfig.data_dir` 由平台层注入 |
| 接收保存目录 | 默认应用专属外部存储；"导出到下载"通过插件走 MediaStore（V0.2） |
| 文件选择 | `tauri-plugin-dialog`（系统文件选择器），替代桌面拖拽 |
| 网络权限 | `AndroidManifest.xml`：`INTERNET`、`ACCESS_NETWORK_STATE`、`CHANGE_WIFI_MULTICAST_STATE` |
| 进程存活 | V0.1 接受切后台可能中断；前台服务在 V0.2 实现 |
| 最低版本 | minSdk 24（Android 7.0） |

## 12. 推荐依赖

| 用途 | crate |
|------|-------|
| 异步运行时 | `tokio` |
| mDNS | `mdns-sd` |
| TLS | `rustls` + `rcgen`（自签名证书） |
| 密钥 | `ed25519-dalek` |
| 哈希 | `blake3` |
| 序列化 | `serde` + `bincode`（线路）/ `serde_json`（UI） |
| 数据库 | `rusqlite`（bundled） |
| 错误 | `thiserror` |
| 日志 | `tracing` + `tracing-subscriber` |
| 路径 | `dirs` |
| ID | `uuid` |
