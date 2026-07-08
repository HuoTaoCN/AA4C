# AA4C 下载中心设计（V0.4）

> 状态：**v2（评审修订，已定稿）**，对应 [ROADMAP.md](ROADMAP.md) V0.4（AA Download）。本文档是落地依据，不含实现代码；实现拆解见 [V0.4_IMPLEMENTATION_PLAN.md](V0.4_IMPLEMENTATION_PLAN.md)（D1 细化到步骤级）。
> **v1 → v2 评审修订的四个实质问题**：① aria2 官方 release 实际上**不提供 macOS / Linux x86_64 预编译二进制**（只有 Windows + Android aarch64 + 源码，已对官方 release 资产逐项核实），v1"直接下载官方产物"对三分之二的目标平台不成立，改为自建引擎构建流水线（§3.1）；② v1 完全没有回答"应用退出再启动，下载任务怎么办"，补任务持久化与启动对账（§3.4）；③ `--rpc-secret` 走命令行参数会被本机任意进程经 `ps`/WMI 看到，直接推翻 v1 §7 自己写的"拿不到密钥就调不了"，改走 data_dir 下 0600 权限的配置文件（§3.1/§7）；④ v1 默认下载目录"save_dir 同级的 Downloads 子目录"表述自相矛盾，且若按"子目录"理解会落进 Inbox 索引根（=整个 save_dir，递归扫描），等于"下载即自动分享给所有完全信任设备"，改为系统下载目录 + 范围警示（§5/§7）。另补孤儿进程防护（`stop-with-process`）、端口竞态重试、进度写库节流等小项。
> 关联：产品定位见 [PROJECT_VISION.md](PROJECT_VISION.md) §四.4 / §七 / §十三；架构分层见 [ARCHITECTURE.md](ARCHITECTURE.md)；表结构见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) §4e；界面见 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md)。
> 本次会话确认的三个范围决定：**AA4C 自动打包并管理外部下载引擎的子进程**（而非要求用户自己装好）；**先做 Aria2（HTTP/HTTPS/FTP），qBittorrent（BT/Magnet）后置为独立里程碑**；**V0.4 只覆盖桌面三平台，不含 Android**。三点理由见 §1.1 与 §9。

## 1. 背景与目标

V0.1–V0.3（AA Nearby → AA Sync → AA Connect）解决的都是**设备与设备之间**的文件流动——局域网、跨设备同步、跨互联网连接，内容始终来自"某台已配对的设备"。V0.4 要解决一个不同性质的问题：把**公网上的任意资源**（HTTP/HTTPS/FTP 直链、BT/Magnet）拉进同一个"AA4C 文件空间"，拉下来之后自然可以走已有的同步/分享能力继续流动。这不是新的连接方式，是新的**内容来源**。

四个目标：

1. **统一任务中心**：HTTP/HTTPS/FTP 与后续 BT/Magnet 下载收进同一个任务列表，复用 AA 传输页已经验证过的进度/状态视觉语言（进度条、速度、ETA），不重新发明一套 UI。
2. **不重新发明轮子**：不自研下载引擎、不自研 BT 客户端——包一层成熟、久经考验的外部工具（aria2、qBittorrent），通过 RPC/API 控制。BT 协议栈（DHT、piece 选择、tracker、磁力解析……）本身就是一个足以撑起一整个项目的工作量，自研不符合 [AGENTS.md](AGENTS.md) "简单 > 复杂"的原则。
3. **许可证隔离**：aria2（GPLv2）、qBittorrent（GPLv3）只作为**独立进程**存在，AA4C 只通过网络协议（JSON-RPC / HTTP API）与它们通信，不链接、不嵌入源码——避免 copyleft 传染到 AA4C 自己的 Apache-2.0 代码（PROJECT_VISION.md §十三已经定下这条原则，V0.4 是它第一次真正要落地检验）。
4. **开箱即用**：用户不需要自己安装、配置 aria2 或 qBittorrent——AA4C 打包对应平台的二进制，随应用生命周期自动拉起/退出，界面上感知不到"背后是个独立进程"。

设计原则延续 [AGENTS.md](AGENTS.md)：稳定 > 功能，简单 > 复杂，默认安全；延续 V0.1–V0.3 已经验证过的模式（事件总线驱动 UI、依赖倒置解耦 Core 与具体实现、失败降级而非阻塞启动）而不是另起一套。

### 1.1 范围与阶段划分（已确认）

- **V0.4 内部按里程碑切分**：D1（Aria2 / HTTP-FTP，本文档的实现重点）→ D2（qBittorrent / BT-Magnet）→ D3（统一任务中心打磨）。两个外部依赖一起接入会让第一版的进程管理、错误处理、测试面同时翻倍，参考 V0.3 拆成 C1–C6 分步验收的经验，V0.4 也分步走。
- **子进程由 AA4C 自动管理**：打包对应平台的 aria2c（后续 qBittorrent-nox）二进制，随 Core 启动/关闭自动拉起/终止，不要求用户预先安装或手动配置 RPC 地址。这比"假设用户自己已经装好、只填 RPC 地址"多做了打包与生命周期管理的工作量，换来的是不熟悉这两个工具的用户也能开箱即用——符合 AA4C"不需要注册、登录、账号，连上就能用"的一贯产品姿态。
- **V0.4 只覆盖桌面三平台**（Windows / macOS / Linux），不含 Android。aria2c/qBittorrent 是原生二进制，Android 上的打包、前台服务常驻、电池优化白名单是完全不同的一套问题，留到后续单独评估——同 V0.3 分享链接里程碑把 deep-link 系统注册单独拆出去、不阻塞主里程碑的处理方式一致。

## 2. 架构总览

新 crate `aa4c-download`，与 `aa4c-transfer` 平级、独立于设备身份/配对——下载没有"对端设备"概念，是比 Transfer 更简单、更自包含的一类能力，不需要 mTLS、不需要证书固定。

**关键约束**：子进程的实际拉起动作要用 Tauri 的 `tauri-plugin-shell`（`ShellExt::shell().sidecar(name)`），这个 API 需要 `AppHandle`，是 Tauri 专属能力。而 `aa4c-core` 目前是一个**不依赖 Tauri 的纯 Rust 库**（[ARCHITECTURE.md](ARCHITECTURE.md)："Core 是纯 Rust 库，可被 Tauri（桌面 + 移动）、Docker（HTTP）复用"）——这条边界是 V0.1 起就有的既定原则，不应该因为 V0.4 需要调用一个 Tauri 专属 API 就被打破。

解法复用 C1–C6 里反复验证过的依赖倒置手法（`IncomingPairDispatch` / `RelayDialer` / `PunchDialer` / `ShareResolver` 都是同一个模式）：`aa4c-download` 定义一个 `SidecarSpawner` trait（"替我拉起打包的某个可执行文件、给我一个能终止它/观察它退出的进程句柄"——注意与 aria2c 的**通信**走回环 RPC 而不是 stdin/stdout，这个 trait 只管进程生死，不管数据面），具体实现由 Tauri 壳层注入，基于 `tauri_plugin_shell::ShellExt::sidecar()`。`aa4c-download` 自己不知道、也不需要知道背后是不是 Tauri——将来要在无 GUI 的 Docker/NAS 场景跑 headless Core，注入一个直接 `std::process::Command` 的实现即可，`aa4c-download` 内部逻辑不用改一行。

```
AA4C UI (Vue3)
   │ Tauri IPC
AA4C Core (纯 Rust，不依赖 Tauri)
   │
DownloadService (aa4c-download)
   │  SidecarSpawner  ←── 注入，Tauri 壳层用 tauri-plugin-shell 实现
   │  Aria2Client（JSON-RPC + WebSocket，纯 Rust，不依赖 Tauri，可独立单测）
   ▼
aria2c 子进程（bundled sidecar，只监听 127.0.0.1，随机端口 + 随机密钥）
```

## 3. aria2 集成

### 3.1 进程生命周期

- **打包**：用 Tauri 2 的 sidecar 机制（`tauri.conf.json` 的 `bundle.externalBin`）。每个目标平台/架构各准备一份 aria2c 二进制，按 `aria2c-<target-triple>[.exe]` 命名放进 `src-tauri/binaries/`（如 `aria2c-x86_64-pc-windows-msvc.exe`、`aria2c-aarch64-apple-darwin`、`aria2c-x86_64-unknown-linux-gnu`）。
- **二进制来源（v2 修订）**：aria2 官方 release（最新 1.37.0，2023-11）**只提供 Windows（32/64 位）与 Android aarch64 预编译产物，macOS 与 Linux 只有源码**——v1"从官方 release 下载对应平台产物"对三分之二的目标平台不成立。决定：**自建一条独立的引擎构建流水线**——一次性 workflow（不进每次应用 release），从官方源码仓库的固定 release tag 编译 macOS（x86_64 + aarch64）与 Linux x86_64 静态产物（TLS 后端用 aria2 官方支持的构建配置：macOS AppleTLS、Linux 静态 OpenSSL/musl；Windows 直接取官方 zip 里的 WinTLS 构建），产物连同 SHA-256 上传到本仓库一个专门的 engines release（如 tag `engines/aria2-1.37.0`），**校验和写死进仓库源码**。应用的 release workflow 只做"按写死的校验和下载 + 验证 + 放进 `binaries/`"，不在每次发版时重新编译——引擎版本升级是显式、低频、有校验和 diff 可审的动作，供应链信任锚定在"官方源码 tag + 我们自己的 CI"，不引入第三方二进制分发者。
- **启动**：`Core::start()` 阶段（与传输监听、mDNS 广播等其余服务同一批，见下方"eager vs lazy"的决定）通过注入的 `SidecarSpawner` 拉起。**全部选项写进一个每次启动重新生成的配置文件**（`<data_dir>/aria2.conf`，Unix 上 0600 权限），命令行只传 `--conf-path=<该文件>`——密钥不放命令行是硬要求：命令行参数对本机**任意用户**的进程经 `ps`/WMI 可见，会直接推翻 §7"拿不到密钥就调不了"的隔离声明；而 data_dir 里本来就存着设备私钥，密钥落在同一目录不引入任何新的信任假设。conf 内容（生成逻辑，非字面模板）：
  ```
  enable-rpc=true
  rpc-listen-port=<探测到的空闲端口>
  rpc-listen-all=false
  rpc-secret=<每次启动随机生成>
  dir=<Settings.download_dir>
  stop-with-process=<AA4C 自身 PID>
  save-session=<data_dir>/aria2.session
  input-file=<data_dir>/aria2.session   # 仅当该文件已存在时写入这行
  save-session-interval=30
  continue=true
  ```
  顺带的收益：sidecar 命令行收敛成固定形状（只有 `--conf-path` 一个参数），Tauri 2 shell capability 的参数放行可以用精确匹配，不必开"允许任意参数"的口子。
- **端口占用竞态**："探测到空闲端口"与"aria2c 真正 bind"之间有竞态窗口（aria2 不支持让操作系统自选端口），端口被抢会让 aria2c 启动失败——按"换个端口重拉"处理，有限次重试，与健康检查共用同一套递增退避，不做端口预留。
- **端口与密钥不跨启动持久化**：每次启动随机生成，写进当次 conf（下次启动即整体覆盖）并在内存里传给 `Aria2Client`——同 aa4c-server 中继 token 的思路一致（短生命周期凭证，用完即弃，缩小被动攻击面）。
- **关闭**：`Core::shutdown()` 时终止子进程——先礼后兵，`aria2.shutdown` RPC 优雅关闭（会触发一次 session 保存），超时后强制 kill。**没机会跑 shutdown 的场景**（AA4C 崩溃、被强杀）由上面 conf 里的 `stop-with-process` 兜底：aria2c 监测到宿主进程消失就自行退出——从根上消灭孤儿进程，不需要 PID 文件 + 启动时清扫那套簿记（aria2 手册明确这个选项就是为"被父进程 fork 出来"的嵌入场景设计的）。
- **健康检查**：启动后轮询 `aria2.getVersion` 直到就绪（有限次重试，间隔递增）；探测失败则下载能力整体不可用，但**不阻塞 Core 启动**——同 QUIC 端点/服务器常驻连接等其余能力"绑不上/连不上就优雅降级、不影响主功能"的一贯设计。
- **eager 还是 lazy**：**决定 eager**（Core 启动即拉起，不等用户第一次打开下载页）。理由：aria2c 空闲时资源占用很小；eager 换来"用户点开下载页时永远是就绪状态"的简单体验，也不需要处理"运行中临时拉起、拉起过程中用户又操作了"这类额外状态机，与现有 QUIC 端点/mDNS 广播的启动方式一致。

### 3.2 RPC 通信

- **传输**：JSON-RPC 2.0 **over WebSocket，单连接**——指令的请求/响应与事件通知走同一条连接（aria2 官方支持在 WS 上跑与 HTTP 相同的方法签名）。v2 评审稿写的是"HTTP 发指令 + WS 收事件"，产出实现计划时收敛成 WS 单连接：只引一个依赖（`tokio-tungstenite`），不必为发指令再拉一个 HTTP 客户端（reqwest 依赖树过重，手写 HTTP/1.1 又是无谓代码）；请求按 JSON-RPC id 关联响应，按键控 pending 表做请求-响应配对在代码库里有现成先例（C5 `SignalChannel`）；断线重连只需要管一条连接，重连后跑 §3.4 的对账。
- **鉴权**：每次调用带 `token:<rpc-secret>` 参数（aria2 官方约定的认证方式；`--rpc-user`/`--rpc-passwd` 官方标注即将弃用，不采用）。
- **事件驱动，不轮询**：订阅 `aria2.onDownloadStart` / `onDownloadPause` / `onDownloadStop` / `onDownloadComplete` / `onDownloadError` 五个 WebSocket 通知，收到后立即拉一次 `aria2.tellStatus(gid)` 补全详情、转成 `CoreEvent` 广播——同 AA4C 全局"事件总线驱动 UI，不轮询"的既有风格（mDNS 发现、传输进度都是这个模式）。
- **兜底**：WebSocket 断线重连期间可能漏事件，用一条低频（数秒级）的 `aria2.tellActive` 轮询兜底同步——同 `sync_index`"文件监听 + 定时扫描兜底"的先例，轮询不是主力机制，只防漏、防断线期间状态漂移。
- **用到的方法**：`addUri`（发起下载，支持镜像 URL 数组）、`pause`/`unpause`、`remove`/`forceRemove`（取消）、`tellStatus`（单任务详情）、`tellActive`/`tellWaiting`/`tellStopped`（列表，启动/WS 重连后拿全量对齐用）、`getGlobalStat`（总体速度，任务中心页头部可选展示）。

### 3.3 任务模型映射

- aria2 自己给每个任务生成一个 GID（16 位 hex），**直接当 AA4C 这边 `download_tasks.id` 用**，不另起一个 UUID 做双重映射——减少一层簿记，同 C6 里"token 直接当能力凭证、不二次包装"的同一种"不做无谓间接层"的取舍。
- 状态映射：aria2 的 `active`/`waiting`/`paused`/`error`/`complete`/`removed` 六态，UI 层转成人话（"下载中"/"排队中"/"已暂停"/"失败"/"已完成"/"已取消"），不出现"GID"/"RPC"等技术词（同 `format.ts` 现有 `statusText` 的转译惯例）。

### 3.4 任务持久化与重启恢复（v2 新增）

v1 没有回答"应用退出再启动，进行中的下载怎么办"。答案分两层，职责不重叠：

- **续传数据归 aria2 管**：`save-session` 让 aria2 在退出时（外加每 30s 一次，覆盖崩溃窗口）把未完成任务写进 `aria2.session`，下次启动经 `input-file` 装回。**普通 URI 下载在 session 文件里保存原 GID**（aria2 手册明确保证；只有本地 torrent/metalink 文件这类元数据驱动的下载有 GID 保存的例外——D1 的 HTTP/HTTPS/FTP 直链全部属于"普通 URI"，不受影响）。这正是 §3.3"GID 直接当 `download_tasks.id`"能跨重启成立的前提——如果 GID 每次重启都变，那个决定就得推翻。半成品文件的字节级续传由 aria2 自己的 `.aria2` 控制文件 + `continue=true` 负责，AA4C 不掺和。
- **任务记录归 AA4C 管，启动时对账**：aria2c 健康检查通过后拉一次 `tellActive`/`tellWaiting`/`tellStopped` 全量，与 `download_tasks` 表对齐：两边都有 → 以 aria2 为准刷新状态/进度；表里是未完态（active/waiting/paused）但 aria2 里没有（session 文件丢失/损坏/被手动删）→ 标 `error`（转译成"应用重启后任务已丢失，请重新添加"），同 V0.1 起 `restart_marks_stale_tasks_failed` 对 `transfer_tasks` 的既有先例；aria2 里有但表里没有（上次 `addUri` 成功后、写库前恰好崩了）→ 补插一行。对账逻辑幂等，WebSocket 断线重连后跑同一段，不为重连单写一套。

## 4. 数据模型

新表 `download_tasks`（不复用 `transfer_tasks`，理由见 §9）：

```sql
CREATE TABLE download_tasks (
    id                TEXT PRIMARY KEY,          -- aria2 GID，直接复用，不二次映射
    kind              TEXT NOT NULL DEFAULT 'http'
                      CHECK (kind IN ('http','bt')),   -- 'bt' 留给 D2（qBittorrent）
    url               TEXT NOT NULL,              -- 原始 URL / magnet URI
    save_path         TEXT,                       -- 落盘路径（完成后由 aria2 汇报回填）
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

- 没有 `peer_device_id` 之类的设备关联字段——下载任务天然没有"对端设备"，这也是不复用 `transfer_tasks` 的直接原因（该表的 `peer_device_id` 有 `REFERENCES devices(id)` 外键，是"peer 必然是已配对设备"这个假设的产物，见 PROTOCOL.md §16 / DATABASE_SCHEMA.md §4c.1 记录的 C6 教训——不重蹈覆辙的最简单办法就是不共用这张表）。
- 速度/ETA 不落库，只在事件里带、前端本地维护——同 `transfer_tasks` 不存 speed 字段的既有先例。
- `downloaded_bytes`/`updated_at` **不随每个进度 tick 写库**：状态迁移（开始/暂停/完成/失败）必写，进行中按数秒级节流写一次——进度的实时性由事件负责，库里的值只服务于重启后的列表恢复显示（§3.4 对账时反正会被 aria2 的真实状态刷新），允许略旧。

## 5. Core 集成

- `aa4c_types::CoreEvent` 追加（只追加变体，不改现有）：`DownloadProgress{task_id, downloaded_bytes, total_bytes, speed_bps}` / `DownloadDone{task_id, save_path}` / `DownloadFailed{task_id, error}`——形状照抄 `TransferProgress`/`TransferDone`/`TransferFailed`，前端能直接复用同一套卡片组件与节流写库逻辑（`Progress` 结构体的模式）。
- `Settings` 追加 `download_dir: Option<String>`：默认取**系统下载目录**（`dirs::download_dir()`，与浏览器落点一致的直觉），**必须在 `save_dir` 子树之外**——这不是风格偏好：Inbox 的索引根就是整个 `save_dir`（`Core::start()` 里 `ensure_inbox_scope(&save_dir)`，扫描器递归遍历），落进去的任何文件都会被自动索引、对所有完全信任设备可见可拉取，等于"下载即分享"（v1 写的"save_dir 同级的 Downloads 子目录"表述自相矛盾，按"子目录"理解恰好踩中这一点，v2 修正）。默认 `save_dir` 是 `~/Downloads/AA4C`，系统下载目录 `~/Downloads` 是它的父目录、不在其子树内，隔离成立。用户手动改 `download_dir` 时，设置页对"目标落在任一同步范围内"的选择给出明确警示（说明会被同步出去，**不硬禁**——用户明白后果后有权把下载目录当同步源用）。
- `Core` 新增 `download: Arc<DownloadService>` 字段，与现有 `transfer`/`pairing` 并列；`orchestrate.rs` 新增编排方法：`add_download(url) -> task_id`、`pause_download`/`resume_download`/`cancel_download`、`list_downloads`。
- Tauri 新增对应 Command（`add_download`/`pause_download`/`resume_download`/`cancel_download`/`list_downloads`）+ 事件转发（`event_payload` 加三个新分支）——同 C1–C6 每次新增能力时的既有接线流程，不再赘述。

## 6. UI

- 「下载」页（`nav.ts`/路由已有占位）：顶部一个链接输入框（粘贴 HTTP/HTTPS/FTP 直链；D2 起也接受 magnet）+「开始下载」按钮；下方任务列表复用 `TransferCard.vue` 的视觉语言（进度条、速度、ETA 全部照搬 `humanBytes`/`humanSpeed`/`etaText`）。
- 每条任务：暂停/继续、取消、完成后「打开所在文件夹」（同现有 toast 的 `openDir` 惯例）。
- 术语人话：不出现 aria2/RPC/GID 等技术词；错误统一转译（同 `format.ts` 的 `errorText` 惯例）。

## 7. 安全与合规考量

| 议题 | 对策 |
|------|------|
| GPL 许可证传染 | aria2（GPLv2）/ qBittorrent（GPLv3，D2）只作为**独立进程**存在，AA4C 只通过网络协议（JSON-RPC / HTTP API）与之通信，不链接、不嵌入源码；随安装包分发时附带对应 LICENSE/NOTICE 文件，明确标注这是打包的第三方运行时依赖、许可证与 AA4C 自身代码（Apache-2.0）分开标注 |
| RPC 暴露面 | `rpc-listen-all` 保持关闭（只绑 127.0.0.1），`rpc-secret` 每次启动随机生成；**密钥不走命令行参数**——命令行对本机任意用户的进程经 `ps`/WMI 可见，v1 的"拿不到密钥就调不了"在那个方案下不成立——改写进 data_dir 下 0600 权限的 conf 文件（§3.1）。诚实的边界声明：同一用户的其他进程本来就读得到 data_dir 里的一切（包括设备私钥），这不是本设计能扩大的边界；修掉的是"连**其他用户**的进程都能从进程列表里直接看到密钥"这个更大的洞 |
| 下载内容不受信 | 下载的是用户主动提供的任意公网 URL，AA4C 不做内容扫描/校验（同浏览器下载的信任模型，不是 AA4C 新引入的风险面）；aria2 能访问的网络范围与本机用户本身能访问的范围相同，不构成新的攻击面 |
| 下载目录默认隔离 | `download_dir` 是独立设置项，不自动并入任何同步范围/Inbox——避免用户下载的内容未经确认就被分享给已配对设备 |
| 子进程崩溃/被杀 | aria2c 异常退出时下载能力整体不可用，但不影响 AA4C 其余功能（同其余可选能力的一贯降级设计）；D1 先不做自动重启，观察实际故障率后再决定要不要加。反方向（AA4C 崩溃/被强杀、来不及跑 shutdown）由 `stop-with-process=<AA4C PID>` 兜底，aria2c 自行退出，不留孤儿进程（§3.1） |
| 上游维护风险 | aria2 最新 release 是 1.37.0（2023-11），发版节奏明显放缓。对策：引擎版本由我们钉死并自建产物（§3.1），升级是显式受控动作、不自动跟随上游；RPC 只绑回环 + 密钥隔离，网络暴露面本就极小；`DownloadService` 对上层只暴露任务模型，引擎在背后可整体替换（D2 接 qBittorrent 本身就会验证这层抽象的可替换性） |

## 8. 里程碑切分

1. **D1 — Aria2 集成（HTTP/HTTPS/FTP，本文档的实现重点）**：sidecar 打包 + 生命周期管理（`SidecarSpawner` 注入点）+ `Aria2Client`（RPC + WebSocket 事件）+ `download_tasks` 表 + Core 集成（`CoreEvent`/Command）+「下载」页基础 UI（链接输入 + 任务列表 + 暂停/取消/打开文件夹）。
2. **D2 — qBittorrent 集成（BT/Magnet）**：qbittorrent-nox sidecar；Web API 鉴权是 cookie session（与 aria2 的 token 模型完全不同，需要单独适配，不能照搬 `Aria2Client` 的鉴权逻辑）；`download_tasks.kind='bt'` 分支；磁力链接输入 + 做种数/连接数等 BT 专属信息展示。
3. **D3 — 统一任务中心打磨**：设置页新增「下载」区块（下载目录、并发数/限速透传给 aria2/qBittorrent 的 options）；D1+D2 任务在同一个列表里按时间统一排序；批量操作（全部暂停/清除已完成记录）。

## 9. 已确认的设计细节

| 议题 | 决定 | 落点 |
|------|------|------|
| 下载引擎实现路径 | 不自研，包外部成熟工具（aria2/qBittorrent），通过 RPC/API 调用、不链接源码——避免 GPL 传染，同时省下重新实现下载协议栈/BT 协议栈的巨大工作量 | §1，PROJECT_VISION.md §十三 |
| 子进程管理 | AA4C 自动打包 + 生命周期管理（Tauri sidecar），不要求用户自己装、自己配 RPC 地址 | §1.1，§3.1 |
| 首个里程碑范围 | 先做 Aria2（HTTP/HTTPS/FTP），qBittorrent（BT/Magnet）后置为独立里程碑 D2——两个外部依赖一起接入会让第一版的进程管理/错误处理/测试面同时翻倍 | §1.1，§8 |
| V0.4 平台范围 | 仅桌面三平台，不含 Android——原生二进制打包/常驻在移动端是完全不同的问题，留到后续单独评估 | §1.1 |
| DownloadService 与 Tauri 解耦 | 新增 `SidecarSpawner` trait（同 C1–C6 一路建立的依赖倒置先例：`RelayDialer`/`PunchDialer`/`ShareResolver`），`aa4c-download` 不直接依赖 `tauri-plugin-shell`，保持 Core 层"纯 Rust、可被 Docker/无 GUI 场景复用"的既定原则不被打破 | §2 |
| 任务表不复用 `transfer_tasks` | 下载没有"对端设备"概念，`transfer_tasks.peer_device_id` 的外键假设不适用；新建 `download_tasks` 独立表，避免重蹈 C6 那次外键假设冲突的覆辙 | §4 |
| 下载目录与同步范围默认隔离 | `download_dir` 是独立设置项，不自动并入任何同步范围/Inbox——下载的是不受信的公网内容，是否同步给其他设备由用户自己决定 | §5，§7 |
| aria2 GID 直接当任务 id | 不做二次 UUID 映射，减少一层簿记（同 C6 token 直接当能力凭证的取舍） | §3.3 |
| 进程启动策略 | eager（Core 启动即拉起），不做懒启动——同现有服务的启动方式一致，代码路径更简单，运行时资源成本可忽略 | §3.1 |
| RPC 端口/密钥生命周期 | 不跨启动持久化，每次启动随机生成（写入当次 conf，下次启动整体覆盖）——同中继 token 的短生命周期凭证思路 | §3.1，§7 |
| 引擎二进制来源（v2） | 官方 release 只有 Windows 产物；macOS/Linux 从官方源码固定 tag 自建静态产物，一次性 engines release + SHA-256 校验和写死进仓库，应用发版只做下载+校验，不引入第三方二进制分发者 | §3.1 |
| RPC 密钥传递方式（v2） | 不走命令行（`ps`/WMI 对任意用户可见），全部选项含密钥写进 data_dir 下 0600 的 conf 文件，命令行只有 `--conf-path`（顺带让 Tauri capability 参数放行可用精确匹配） | §3.1，§7 |
| 任务跨重启恢复（v2） | 续传数据归 aria2（`save-session`/`input-file`，普通 URI 下载 GID 跨重启不变——"GID=id"决定成立的前提）；任务记录归 AA4C，启动/WS 重连后 `tellActive/Waiting/Stopped` 全量对账，孤儿未完记录标失败（同 `restart_marks_stale_tasks_failed` 先例） | §3.4 |
| 孤儿进程防护（v2） | conf 里写 `stop-with-process=<AA4C PID>`：宿主进程消失（含崩溃/强杀）时 aria2c 自行退出；不做 PID 文件 + 启动清扫 | §3.1，§7 |
| 默认下载目录（v2） | 系统下载目录（`dirs::download_dir()`），必须在 `save_dir` 子树之外——Inbox 索引根=整个 `save_dir`，落进去=自动分享给完全信任设备；用户改到同步范围内时警示不硬禁 | §5，§7 |
| RPC 传输载体（v2.1） | JSON-RPC over WebSocket **单连接**（指令与事件同一条），不为发指令引入 HTTP 客户端依赖；id 关联 pending 表同 `SignalChannel` 先例 | §3.2 |

## 仍待实现 / 后续

qBittorrent Web API 的具体鉴权细节（cookie session，需要单独设计；另注意 qBittorrent 没有 `stop-with-process` 等价物，孤儿进程防护要另想办法）；限速/并发数等选项具体要透传 aria2/qBittorrent 的哪些 option、设置页字段怎么设计；下载失败的自动重试策略；子进程崩溃后的自动重启策略；"下载目录落在同步范围内"警示的具体交互（D3 设置页一起做）；引擎版本升级的操作流程文档化（engines release 的产出步骤，做进 CONTRIBUTING 或脚本注释）；Android 平台的下载能力方案（很可能是完全不同的技术路径，比如系统 DownloadManager，而不是 bundled 二进制，需要单独评估）；S3 协议支持（PROJECT_VISION.md 提到但未细化，大概率需要凭证管理，单独评估，不在 D1–D3 范围内）。
