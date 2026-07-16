# AA4C 下载中心设计（V0.4）

> 状态：**里程碑 D1、D2 均已实现**（D1 对应 [V0.4_IMPLEMENTATION_PLAN.md](V0.4_IMPLEMENTATION_PLAN.md) D1 的 10 个步骤全部完成；D2 的孤儿进程防护/子进程管理/RPC 客户端/Core 编排+前端 UI 全部落地并有真实进程集成测试覆盖，唯一未完成的是**引擎二进制的正式打包分发管线**——`engines.yml` 的 Transmission 构建腿、`tauri.conf.json` 的 `externalBin` 声明、`fetch-engines.sh` 的对应条目都还没做，这部分被有意排到最后，见 §3.6.5 与"仍待实现"）。D3（任务中心打磨）仍是设计稿。实现相对设计的偏差记在 §3.5（D1）/§3.6.6（D2）。
> **v2 → v3 修订（D2 动手前）的两个决定**：① **D2 的 BT 引擎从 qBittorrent 换成 Transmission**——按 D1 教训（先逐项核实二进制分发再定方案）实际调查的结果：qBittorrent 的 headless（nox）构建在 Windows 上官方与社区**都不存在**、macOS 官方没有且上游对要不要提供仍有争议，等于两个平台要从零攻坚没有先例的 Boost+libtorrent 构建；而 Transmission 官方 Windows MSI 自带 `transmission_daemon.exe`（已实际拆包核实），macOS/Linux 的 Homebrew core formula 就是按 `-DENABLE_DAEMON=ON` 构建的（双架构 bottle 齐全），源码 CMake 路线可直接进 engines.yml。RPC 也更简单（header token vs cookie session）。详见 §3.6 与 §9。② **预留 Lua 插件系统的设计边界**（§10）：私有 Tracker/PT 站、搜索、自动分类等站点化长尾需求走用户可写的 Lua 插件，适用于全部下载类型而非 BT 专属；V0.4 不实现，但 D2/D3 的接缝现在就要按 §10 的约束留好。
> **v1 → v2 评审修订的四个实质问题**：① aria2 官方 release 实际上**不提供 macOS / Linux x86_64 预编译二进制**（只有 Windows + Android aarch64 + 源码，已对官方 release 资产逐项核实），v1"直接下载官方产物"对三分之二的目标平台不成立，改为自建引擎构建流水线（§3.1）；② v1 完全没有回答"应用退出再启动，下载任务怎么办"，补任务持久化与启动对账（§3.4）；③ `--rpc-secret` 走命令行参数会被本机任意进程经 `ps`/WMI 看到，直接推翻 v1 §7 自己写的"拿不到密钥就调不了"，改走 data_dir 下 0600 权限的配置文件（§3.1/§7）；④ v1 默认下载目录"save_dir 同级的 Downloads 子目录"表述自相矛盾，且若按"子目录"理解会落进 Inbox 索引根（=整个 save_dir，递归扫描），等于"下载即自动分享给所有完全信任设备"，改为系统下载目录 + 范围警示（§5/§7）。另补孤儿进程防护（`stop-with-process`）、端口竞态重试、进度写库节流等小项。
> 关联：产品定位见 [PROJECT_VISION.md](PROJECT_VISION.md) §四.4 / §七 / §十三；架构分层见 [ARCHITECTURE.md](ARCHITECTURE.md)；表结构见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) §4e；界面见 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md)。
> v1 确认的三个范围决定：**AA4C 自动打包并管理外部下载引擎的子进程**（而非要求用户自己装好）；**先做 Aria2（HTTP/HTTPS/FTP），BT/Magnet 后置为独立里程碑 D2**（v1 时选型 qBittorrent，v3 换成 Transmission，见上）；**V0.4 只覆盖桌面三平台，不含 Android**。理由见 §1.1 与 §9。

## 1. 背景与目标

V0.1–V0.3（AA Nearby → AA Sync → AA Connect）解决的都是**设备与设备之间**的文件流动——局域网、跨设备同步、跨互联网连接，内容始终来自"某台已配对的设备"。V0.4 要解决一个不同性质的问题：把**公网上的任意资源**（HTTP/HTTPS/FTP 直链、BT/Magnet）拉进同一个"AA4C 文件空间"，拉下来之后自然可以走已有的同步/分享能力继续流动。这不是新的连接方式，是新的**内容来源**。

四个目标：

1. **统一任务中心**：HTTP/HTTPS/FTP 与后续 BT/Magnet 下载收进同一个任务列表，复用 AA 传输页已经验证过的进度/状态视觉语言（进度条、速度、ETA），不重新发明一套 UI。
2. **不重新发明轮子**：不自研下载引擎、不自研 BT 客户端——包一层成熟、久经考验的外部工具（aria2、Transmission），通过 RPC/API 控制。BT 协议栈（DHT、piece 选择、tracker、磁力解析……）本身就是一个足以撑起一整个项目的工作量，自研不符合 [AGENTS.md](AGENTS.md) "简单 > 复杂"的原则。
3. **许可证隔离**：aria2（GPLv2）、Transmission（GPL-2.0/3.0 双授权）只作为**独立进程**存在，AA4C 只通过网络协议（JSON-RPC / HTTP API）与它们通信，不链接、不嵌入源码——避免 copyleft 传染到 AA4C 自己的 Apache-2.0 代码（PROJECT_VISION.md §十三已经定下这条原则，V0.4 是它第一次真正要落地检验）。
4. **开箱即用**：用户不需要自己安装、配置 aria2 或 Transmission——AA4C 打包对应平台的二进制，随应用生命周期自动拉起/退出，界面上感知不到"背后是个独立进程"。

设计原则延续 [AGENTS.md](AGENTS.md)：稳定 > 功能，简单 > 复杂，默认安全；延续 V0.1–V0.3 已经验证过的模式（事件总线驱动 UI、依赖倒置解耦 Core 与具体实现、失败降级而非阻塞启动）而不是另起一套。

### 1.1 范围与阶段划分（已确认）

- **V0.4 内部按里程碑切分**：D1（Aria2 / HTTP-FTP，已实现）→ D2（Transmission / BT-Magnet，§3.6）→ D3（统一任务中心打磨）。两个外部依赖一起接入会让第一版的进程管理、错误处理、测试面同时翻倍，参考 V0.3 拆成 C1–C6 分步验收的经验，V0.4 也分步走。Lua 插件系统（§10）是 V0.4 之后的独立里程碑，本版只预留接缝。
- **子进程由 AA4C 自动管理**：打包对应平台的 aria2c（D2 起加 transmission-daemon）二进制，随 Core 启动/关闭自动拉起/终止，不要求用户预先安装或手动配置 RPC 地址。这比"假设用户自己已经装好、只填 RPC 地址"多做了打包与生命周期管理的工作量，换来的是不熟悉这两个工具的用户也能开箱即用——符合 AA4C"不需要注册、登录、账号，连上就能用"的一贯产品姿态。
- **V0.4 只覆盖桌面三平台**（Windows / macOS / Linux），不含 Android。aria2c/transmission-daemon 是原生二进制，Android 上的打包、前台服务常驻、电池优化白名单是完全不同的一套问题，留到后续单独评估——同 V0.3 分享链接里程碑把 deep-link 系统注册单独拆出去、不阻塞主里程碑的处理方式一致。

## 2. 架构总览

新 crate `aa4c-download`，与 `aa4c-transfer` 平级、独立于设备身份/配对——下载没有"对端设备"概念，是比 Transfer 更简单、更自包含的一类能力，不需要 mTLS、不需要证书固定。

**关键约束**：子进程的实际拉起动作要用 Tauri 的 `tauri-plugin-shell`（`ShellExt::shell().sidecar(name)`），这个 API 需要 `AppHandle`，是 Tauri 专属能力。而 `aa4c-core` 目前是一个**不依赖 Tauri 的纯 Rust 库**（[ARCHITECTURE.md](ARCHITECTURE.md)："Core 是纯 Rust 库，可被 Tauri（桌面 + 移动）、Docker（HTTP）复用"）——这条边界是 V0.1 起就有的既定原则，不应该因为 V0.4 需要调用一个 Tauri 专属 API 就被打破。

解法复用 C1–C6 里反复验证过的依赖倒置手法（`IncomingPairDispatch` / `RelayDialer` / `PunchDialer` / `ShareResolver` 都是同一个模式）：`aa4c-download` 定义一个 `SidecarSpawner` trait（"替我拉起打包的某个可执行文件、给我一个能终止它/观察它退出的进程句柄"——注意与 aria2c 的**通信**走回环 RPC 而不是 stdin/stdout，这个 trait 只管进程生死，不管数据面），具体实现由 Tauri 壳层注入，基于 `tauri_plugin_shell::ShellExt::sidecar()`。`aa4c-download` 自己不知道、也不需要知道背后是不是 Tauri——将来要在无 GUI 的 Docker/NAS 场景跑 headless Core，注入一个直接 `std::process::Command` 的实现即可，`aa4c-download` 内部逻辑不用改一行。

```
AA4C UI (Vue3)
   │ Tauri IPC
AA4C Core (纯 Rust，不依赖 Tauri)
   │                          ┌─ PluginHost（Lua，§10 预留，V0.4 不实现）
DownloadService (aa4c-download)
   │  SidecarSpawner  ←── 注入，Tauri 壳层用 tauri-plugin-shell 实现
   │  Aria2Client（JSON-RPC over WebSocket，D1 已实现）
   │  TransmissionClient（HTTP RPC + session-id 握手，D2，§3.6）
   ▼
aria2c / transmission-daemon 子进程
（bundled sidecar，只监听 127.0.0.1，随机端口 + 随机凭据）
```

## 3. 下载引擎集成

§3.1–§3.5 是 aria2（D1，已实现）；§3.6 是 Transmission（D2，v3 设计稿）。两个引擎共用同一套外围机制：`SidecarSpawner` 拉起、conf/settings 文件每次启动重写（凭据不进命令行）、只绑回环、启动对账、`reconcile()` 幂等同步、`download_tasks` 同一张表。

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

### 3.5 D1 实现偏差（相对本设计定稿）

- **单线程 actor 模型**：`DownloadService` 内部不是"锁保护共享状态"，而是一个独占持有连接（子进程句柄 + RPC 客户端）的后台任务，公开方法（`add`/`pause`/`resume`/`cancel`）通过 channel 发命令、等回复。好处是"服务当前不可用"有唯一判定点——channel 发送失败 = actor 已退出，不需要额外的健康标志位。
- **事件通知 + 轮询兜底合并成一个函数**：设计稿里"收到 WS 通知 → 精细 `tellStatus(gid)`"与"低频轮询兜底"是两套逻辑；实现收敛成一个 `reconcile()`：不管是通知触发还是定时器触发，都拉一次 `tellActive`/`tellWaiting`/`tellStopped` 全量、按"状态/进度真的变了才写库和广播"做幂等处理。牺牲一点 RPC 精确性换实现简单——个人量级的任务列表下这点开销可忽略，也顺带让"启动对账"“断线重连对账”“运行时兜底”变成同一段代码，不用维护三套相似逻辑。
- **`SHUTDOWN_GRACE` 定为 5 秒（不是随便选的）**：实测发现 aria2 的 `aria2.shutdown` RPC（区别于 `forceShutdown`）内部会等**约 3 秒**才真正退出（日志明确打印「3 second(s) has passed. Stopping application.」），这是 aria2 自己的行为，不是我们能改的；`SHUTDOWN_GRACE` 必须明显长于这 3 秒，否则我们自己的强杀会抢在 aria2 完成 session 落盘之前发生，直接违背"先礼后兵"的本意——这是文档评审阶段没有预料到的细节，靠真实调试才发现。
- **`EngineChild`/`SidecarSpawner` 拆两个 trait**：设计稿把"拉起子进程"当一个整体职责；实现里拆成 `SidecarSpawner::spawn()`（拿到句柄）与 `EngineChild::kill()`（终止），因为 Tauri 的 `CommandChild`（同步 `kill()`）与 `tokio::process::Child`（异步 `kill()`）接口形状不同，拆开后两种壳层实现都能干净适配。
- **`Core.download` 是 `Option`，不是必然存在**：`CoreConfig.download_spawner: Option<Arc<dyn SidecarSpawner>>`，桌面壳层注入、Android 等未接入平台留 `None`——`None` 与"注入了但 aria2c 启动失败"是两种不同的不可用，前端统一收到 `Unavailable` 错误码，不需要区分。
- **人工走查中发现并验证**（V0.4_IMPLEMENTATION_PLAN.md D1 步骤 9）：Tauri capability 权限配置（`shell:allow-execute` + `sidecar:true` + 参数 `validator` 正则）与设计一致、真实跑通；`stop-with-process` 在真实的 `tauri dev` 热重载场景下（进程被替换三次）均正确避免了 aria2c 孤儿进程累积；顺带发现一个真实 bug——`aa4c-core` 的下载端到端测试没有隔离下载目录，实际下载文件落进了开发机真实的系统 Downloads 目录，已修复（测试改为 `Core::start` 之前预置隔离的 `download_dir` 到 settings 表）。

### 3.6 Transmission 集成（D2，v3 设计稿）

#### 3.6.1 为什么从 qBittorrent 换成 Transmission（v3 修订核心）

v1 选 qBittorrent 时没有核实其 headless 构建的三平台分发情况——恰好是 D1 在 aria2 身上踩过的同一类坑（官方 release 缺 2/3 平台的产物，v2 才修正）。D2 动手前按教训逐项核实，结果：

| 平台 | qBittorrent（nox/headless） | Transmission（daemon） |
|------|------------------------------|-------------------------|
| Windows | ❌ 官方与社区**都没有** nox 构建，官方只发 GUI 安装包；从零编译 Boost+libtorrent 的 headless 版本没有任何先例可参照 | ✅ **官方 MSI 自带 `transmission_daemon.exe`**（已实际拆包核实文件表，另含 `transmission_remote.exe` 等 CLI），静默解包即可提取，不执行安装 |
| macOS | ⚠️ 官方无 nox；上游对"要不要提供 macOS nox"仍有争议（PR #6104 未定论）；Homebrew 的 GUI cask 因过不了 Gatekeeper 已被标记 2026-09 停用 | ✅ Homebrew **core** formula `transmission-cli` 就是 `-DENABLE_DAEMON=ON` 构建（arm64 + x86_64 bottle 齐全，活跃维护）；同一套 CMake 配置可直接进 engines.yml 自建 |
| Linux | ✅ 社区 `userdocs/qbittorrent-nox-static` 多架构静态构建，质量好 | ✅ 各发行版标准包 `transmission-daemon`；源码 CMake 路线同上 |

三平台里 Transmission 有两项是官方产物直接提供 headless 二进制，qBittorrent 一项都没有。次要收益：RPC 鉴权是 header token（比 cookie session 简单，不需要维护 cookie jar）；进程更轻。BT 功能面（DHT/PEX/LPD/magnet/加密）两者对本设计的需求无差异。

#### 3.6.2 进程生命周期

复用 §3.1 的全部机制骨架，差异点：

- **前台模式是硬要求**：`transmission-daemon` 默认启动后 fork 到后台、父进程立即退出——`SidecarSpawner` 拿到的句柄会抓错进程（拿到的是即将退出的父进程），`kill()`/退出监测全部失效。必须传 `-f`（`--foreground`）。命令行收敛为固定形状 `transmission-daemon -f --config-dir <data_dir>/transmission`（同 §3.1 只传 `--conf-path` 的思路，Tauri capability 参数放行仍可精确匹配）。
- **配置**：`<config-dir>/settings.json` 每次启动整体重写（同 aria2 conf 先例）：`rpc-bind-address=127.0.0.1`、`rpc-port=<探测的空闲端口>`、`rpc-authentication-required=true`、`rpc-username`/`rpc-password`（每次启动随机生成；Transmission 启动时会把明文密码替换成加盐哈希写回，无碍——我们下次启动整体覆盖）、`download-dir=<Settings.download_dir>`、DHT/PEX/LPD 开启。凭据不进命令行，同 §3.1 的硬要求。注意 Transmission 退出时会把内存中的设置写回 `settings.json`——"每次启动整体重写"的既有决定天然免疫这一点。
- **孤儿进程防护（与 aria2 的关键差异，✅ 已小样验证）**：Transmission **没有** `stop-with-process` 等价物。正常路径靠 `Core::shutdown()`（RPC `session-close` 优雅关闭 → 超时强杀），与 aria2 一致；异常路径（AA4C 崩溃/被强杀，来不及做任何清理）三平台各自的机制均已用一次性 PoC（父进程 `std::process::abort()` 模拟不可控崩溃，验证子进程是否被自动清理）真实验证通过：
  - **Windows**：`CreateJobObjectW` 建 Job → `SetInformationJobObject` 设 `JOBOBJECT_EXTENDED_LIMIT_INFORMATION.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` → `AssignProcessToJobObject` 把子进程句柄归到这个 Job。宿主进程异常终止时 OS 回收其持有的全部句柄（含 Job 句柄），触发"最后一个句柄关闭"条件，内核自动杀光 Job 里的全部进程——不需要宿主自己活着执行任何清理代码。在真实 `windows-latest` GitHub Actions runner 上用 `windows-sys` crate 验证：子进程（`ping -n 600` 占位）在父进程 abort 后确认自动死亡。
  - **Linux**：子进程 `exec` 前（`std::os::unix::process::CommandExt::pre_exec` 钩子内）调用 `prctl(PR_SET_PDEATHSIG, SIGKILL)`，登记"父线程死亡时内核发 SIGKILL 给我"。在真实 `ubuntu-latest` GitHub Actions runner 上验证：子进程（`sleep 600` 占位）在父进程 abort 后确认自动死亡。
  - **macOS**：没有内核层等价机制，用 PID 文件 + 下次启动清扫兜底——写文件时记录 `pid` + 进程启动时间戳（`ps -o lstart=`）+ 进程名（`ps -o comm=`）三元组；下次启动读文件后先核对这三者与当前 `ps` 输出完全一致才动手 kill（防 PID 复用误杀无关进程），核对失败则拒绝清理、静默跳过。本机 bash 脚本验证：正常清扫路径（身份匹配 → 成功清理）与防误杀路径（身份不匹配的"复用同一 PID 的另一进程"场景 → 正确拒绝）均通过。
  
  三个 PoC 的实现要点已固定，D2.4 写生产代码时直接照此实现，不需要再重新设计。
- **健康检查**：轮询 `session-get`（等价于 aria2 的 `getVersion`），退避/降级策略同 §3.1。

#### 3.6.3 RPC 通信

- **传输**：HTTP POST 单端点（`http://127.0.0.1:<port>/transmission/rpc`），请求/响应都是 JSON。**没有 WebSocket、没有事件推送**——Transmission 的 RPC 是纯请求-响应模型。这在 D1 之后不再是缺陷：§3.5 已把"通知触发"与"轮询兜底"收敛成同一个幂等 `reconcile()`，BT 侧直接以数秒级 `torrent-get` 轮询为主路径即可，不需要为"没有事件"另做机制——D1 的收敛决定在这里直接兑现了价值。
- **鉴权**：`X-Transmission-Session-Id` header——首次请求会收到 409 响应、从响应 header 里取 session id，之后每次请求带上；session id 过期再收到 409 就重新取。外加 HTTP Basic（上面 settings.json 里的随机用户名/密码）。
- **HTTP 客户端**：不引 reqwest（依赖树重，D1 已为同样理由弃过一次）——回环、单端点、纯 POST、无 TLS、无重定向，手写极简 HTTP/1.1 客户端（tokio TcpStream，几十行），同 D1 手写测试 HTTP 服务器的先例，放 `aa4c-download` 内部。`TransmissionClient` 独立实现，不硬套 `Aria2Client`（鉴权模型、错误形状、方法命名完全不同，强行抽象只会得到一个两边都别扭的中间层——真正的共享层在 `DownloadService` 的任务模型，不在 RPC 客户端）。
- **用到的方法**：`torrent-add`（`filename` 字段直接放 magnet URI）、`torrent-stop`/`torrent-start`（暂停/继续）、`torrent-remove`（取消，`delete-local-data` 跟随用户选择）、`torrent-get`（全量对账 + 进度）、`session-get`（健康检查）、`session-close`（优雅关闭）、`session-set`（D3 限速透传）。

#### 3.6.4 任务模型映射

- **id**：torrent 的 infohash（`hashString`）直接当 `download_tasks.id`——同"引擎原生 id 不二次映射"的既定原则（§3.3）；infohash 跨重启天然不变，比 aria2 GID 的稳定性论证还简单。`kind='bt'`。
- **入口路由**：`add_download(url)` 按 scheme 分流——`magnet:` → Transmission，其余 → aria2。用户不感知两个引擎的存在（`.torrent` 文件输入留给 D3 或插件阶段，D2 只接 magnet，同 v1 起的范围）。
- **状态映射**：Transmission 的 status（stopped / check-wait / checking / download-wait / downloading / seed-wait / seeding）映射到既有六态：`downloading→active`，`*-wait/checking→waiting`，`stopped` 且未完成 → `paused`，`percentDone==1 → complete`（**做种继续进行**，不因标记完成而停——保种是 BT 生态的基本礼仪，也是私有 Tracker/PT 场景（§10）的硬需求；分享率/做种时长限制 D3 透传设置），错误从 `errorString` 转译。`removed` 由我们的取消动作落库，同 aria2。
- **BT 专属信息**（做种数/连接的 peer 数/分享率）只进事件、不落库——同 speed 不落库的既有先例（§4）。
- **跨重启恢复**：Transmission 原生把 .torrent 元数据与 resume 状态存在 config-dir 下（`torrents/`、`resume/`），不需要 aria2 那套 session 文件机制；启动对账与 §3.4 同构（`torrent-get` 全量 vs 表里 `kind='bt'` 的记录），孤儿未完记录标 `error`、引擎里有表里没有的补插——`reconcile()` 直接扩展，不另写一套。

#### 3.6.5 引擎二进制来源

同 §3.1 的供应链原则（信任锚定在官方产物/官方源码 tag + 我们自己的 CI，校验和写死进仓库）：

- **Windows**：官方 MSI 静默解包提取 `transmission_daemon.exe`（`msiexec /a <msi> /qn TARGETDIR=...` 管理员镜像解包，**不是安装**——不注册服务、不写注册表、不进 PATH；这与 D1 从官方 zip 解 `aria2c.exe` 同级别的官方产物直取，也彻底避开包管理器 shim 那类坑）。注意 MSI 里的可执行文件依赖同目录的 DLL（拆包时一并核实清单）——sidecar 打包要连 DLL 一起进 bundle，`externalBin` 之外用 `bundle.resources` 放伴随文件，这一点 aria2（单文件静态）没有，是 D2 新增的打包差异点，实现时实测。
- **macOS / Linux**：engines.yml 加 transmission 构建腿，CMake `-DENABLE_DAEMON=ON -DENABLE_CLI=OFF -DENABLE_UTILS=OFF -DENABLE_QT=OFF -DENABLE_GTK=OFF -DENABLE_MAC=OFF -DENABLE_TESTS=OFF -DENABLE_NLS=OFF`（照 Homebrew formula 的既验证配置裁剪），依赖 libevent/libpsl/miniupnpc/curl（Linux 另需 openssl/zlib）。macOS 双架构各用原生 runner、Linux 尽量静态——**全部按 aria2 首跑的教训预设**（runner 标签先核实存活、apt 包列表按报错补、静态库缺失时评估 `--without-*` 式裁剪），第一次跑通前不填校验和。
- 产物进同一个 engines release 体系（如 `engines/transmission-<version>`），`scripts/fetch-engines.sh` 扩展第二个引擎条目。

#### 3.6.6 D2 实现偏差（相对本节设计定稿）

- **孤儿进程防护三条路径均已用真实环境 PoC 验证**（不是纸面推导）：Windows `CreateJobObjectW`+`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`+`AssignProcessToJobObject`（按 PID 事后归属，`ProcessSpawner`/`TauriSidecarSpawner` 两条路径统一适用，不需要 spawn 时注入钩子）在真实 `windows-latest` CI runner 上验证通过；Linux `pre_exec` 内 `prctl(PR_SET_PDEATHSIG, SIGKILL)`——**只有 `ProcessSpawner` 能用**，`tauri_plugin_shell::process::CommandChild` 查证后只公开 `write`/`kill`/`pid` 三个方法，没有 spawn 时注入钩子的余地，Tauri 生产路径的 Linux 桌面因此退化用 PID 文件方案（同 macOS）——在真实 `ubuntu-latest` CI runner 上验证通过；macOS（含 Tauri-Linux 兜底）PID 文件+身份校验（pid+启动时间+进程名）本机验证过正常清扫与防误杀两条路径。
- **`SidecarSpawner::spawn` 签名从单参数泛化成 `args: &[String]`**：设计稿假设的"跟 aria2 一样收敛成单参数替换"对 Transmission 不成立——它的命令行是 `-f --config-dir=<path>` 两个参数，不是一个。`ProcessSpawner`/`TauriSidecarSpawner`（后者构造时改传 `sidecar_name`，因为现在有两个不同名字的 sidecar）两个实现同步改了，aria2 侧调用变成传 `&["--conf-path=..."]`，行为不变。`EngineChild` 新增 `pid()` 方法（孤儿防护需要）。
- **`DownloadService` 内部是两个独立 actor，不是一个**：aria2 一个、Transmission 一个，各自的可用性完全独立（一个引擎挂了不影响另一个）。`add()` 按 URL scheme（`magnet:` 前缀）分流；`pause`/`resume`/`cancel` 按任务 id 长度分流（**不是巧合**：aria2 GID 固定 16 位十六进制、BT infohash 固定 40 位十六进制，协议本身决定的固定长度，比每次查一遍数据库拿 `kind` 更省一次往返）。
- **Transmission 没有持久连接可以监听"断开"**（不同于 aria2 的 WebSocket）：`torrent-get` 轮询本身就是唯一的存活信号，`bt_reconcile()` 因此**不吞错误**（不同于 aria2 侧 `reconcile()` 可以依赖 WS `closed()` 事件区分断线），RPC 调用失败原样向上传播，由 actor loop 数连续失败次数（`BT_FAILURE_THRESHOLD=5`），连续失败达到阈值才判定 BT 能力在本次会话内不可用——避免单次网络抖动就整体降级。
- **`CoreEvent::DownloadProgress` 追加三个可选字段**（`seeders`/`peers`/`ratio`，`skip_serializing_if` 让 HTTP 任务的 JSON 里这三个 key 直接不出现，不是出现成 `null`）而不是新开一个 `BtProgress` 变体——让 D1/D2 任务在前端走同一条进度处理路径，直接对应 D3「统一任务中心」的目标；`seeders` 映射 Transmission 的 `peersSendingToUs`，`peers` 映射 `peersConnected`——不是协议原生的"做种者"概念（BT swarm 里"谁是纯种子"没有单一 RPC 字段直接给出），是一个实用近似，D3 打磨阶段如果需要更精确的口径再调整。
- **孤儿记录补插时补不上原始 magnet**：`bt_reconcile()` 发现"引擎有、表里没有"（`add_bt_download` 写库前崩溃的窗口，或用户用别的客户端往同一个 daemon 加了任务）时，退化用 `magnet:?xt=urn:btih:<hash>` 占位（没有 `dn=` 显示名），不影响任务可操作性，只是前端标题会退化显示成截短的 infohash。
- **端到端测试覆盖到"真实进程间路由正确"，不覆盖"真实下载完成"**：`aa4c-core` 新增 `bt_download_routes_through_core_orchestration`，用真实 `aria2c`+`transmission-daemon` 验证 `Core::add_download` 按 scheme 正确路由、任务落库为 `kind: Bt`（id=40 位 infohash）、暂停/继续/取消全部生效。**不测完整下载落盘**——BT 需要真实 peer/tracker 连通性，本地做种测试基础设施本身是个不小的工程量（另起一个 daemon 当种子端+真实 `.torrent` 文件+处理 DHT/PEX 在纯回环环境下不一定能互相发现），同 C5 NAT 打洞的处理先例（CI 只测接线，真实下载场景留给人工走查）。
- **引擎二进制正式打包分发管线未做**（§3.6.5 描述的 engines.yml Transmission 构建腿、`tauri.conf.json` 的 `bundle.externalBin`/`bundle.resources` 声明、`fetch-engines.sh` 的对应条目、`ci.yml` 的 transmission 安装/MSI 解包步骤已经有了但只服务于测试）：这是有意的顺序决定，同 D1 把"引擎二进制流水线"排在代码之后的既定原则（V0.4_IMPLEMENTATION_PLAN.md）——`ci.yml` 已经验证过 Windows 官方 MSI 静默解包提取的方法可行，`apps/desktop/src-tauri/capabilities/default.json` 已经加了 `transmission-daemon` sidecar 的 `shell:allow-execute` 权限（`-f`/`--config-dir=.+` 两个参数位分别校验），代码路径完全就绪，只差正式打包这一步——`CoreConfig.bt_spawner` 已经在桌面壳层注入（`desktop_download_spawner(app, "transmission-daemon")`），只是 sidecar 二进制实际不存在时会在运行时优雅降级成 BT 能力不可用，不会崩溃或阻塞其余功能。

## 4. 数据模型

新表 `download_tasks`（不复用 `transfer_tasks`，理由见 §9）：

```sql
CREATE TABLE download_tasks (
    id                TEXT PRIMARY KEY,          -- 引擎原生 id：aria2 GID / BT infohash，不二次映射
    kind              TEXT NOT NULL DEFAULT 'http'
                      CHECK (kind IN ('http','bt')),   -- 'bt' 留给 D2（Transmission，§3.6）
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
- 速度/ETA 不落库，只在事件里带、前端本地维护——同 `transfer_tasks` 不存 speed 字段的既有先例；BT 的做种数/peer 数/分享率同理（§3.6.4）。
- 插件系统（§10）将来需要的 `category` 列**现在不加**——SQLite `ALTER TABLE ADD COLUMN` 是低成本迁移，等插件里程碑真正定稿再加，不做推测性 schema 设计。
- `downloaded_bytes`/`updated_at` **不随每个进度 tick 写库**：状态迁移（开始/暂停/完成/失败）必写，进行中按数秒级节流写一次——进度的实时性由事件负责，库里的值只服务于重启后的列表恢复显示（§3.4 对账时反正会被 aria2 的真实状态刷新），允许略旧。

## 5. Core 集成

- `aa4c_types::CoreEvent` 追加（只追加变体，不改现有）：`DownloadProgress{task_id, downloaded_bytes, total_bytes, speed_bps}` / `DownloadDone{task_id, save_path}` / `DownloadFailed{task_id, error}`——形状照抄 `TransferProgress`/`TransferDone`/`TransferFailed`，前端能直接复用同一套卡片组件与节流写库逻辑（`Progress` 结构体的模式）。
- `Settings` 追加 `download_dir: Option<String>`：默认取**系统下载目录**（`dirs::download_dir()`，与浏览器落点一致的直觉），**必须在 `save_dir` 子树之外**——这不是风格偏好：Inbox 的索引根就是整个 `save_dir`（`Core::start()` 里 `ensure_inbox_scope(&save_dir)`，扫描器递归遍历），落进去的任何文件都会被自动索引、对所有完全信任设备可见可拉取，等于"下载即分享"（v1 写的"save_dir 同级的 Downloads 子目录"表述自相矛盾，按"子目录"理解恰好踩中这一点，v2 修正）。默认 `save_dir` 是 `~/Downloads/AA4C`，系统下载目录 `~/Downloads` 是它的父目录、不在其子树内，隔离成立。用户手动改 `download_dir` 时，设置页对"目标落在任一同步范围内"的选择给出明确警示（说明会被同步出去，**不硬禁**——用户明白后果后有权把下载目录当同步源用）。
- `Core` 新增 `download: Arc<DownloadService>` 字段，与现有 `transfer`/`pairing` 并列；`orchestrate.rs` 新增编排方法：`add_download(url) -> task_id`、`pause_download`/`resume_download`/`cancel_download`、`list_downloads`。D2 起 `add_download` 内部按 scheme 分流到两个引擎（§3.6.4），且任务添加路径改走一个**引擎无关的请求描述**中间结构（URL + 请求头 + 引擎选项 + 保存子路径）而非一根字符串直通引擎——这既是接第二个引擎本来就要做的抽象，也是 §10 插件系统 `on_before_add` 钩子的预留接缝。
- Tauri 新增对应 Command（`add_download`/`pause_download`/`resume_download`/`cancel_download`/`list_downloads`）+ 事件转发（`event_payload` 加三个新分支）——同 C1–C6 每次新增能力时的既有接线流程，不再赘述。

## 6. UI

- 「下载」页（`nav.ts`/路由已有占位）：顶部一个链接输入框（粘贴 HTTP/HTTPS/FTP 直链；D2 起也接受 magnet）+「开始下载」按钮；下方任务列表复用 `TransferCard.vue` 的视觉语言（进度条、速度、ETA 全部照搬 `humanBytes`/`humanSpeed`/`etaText`）。
- 每条任务：暂停/继续、取消、完成后「打开所在文件夹」（同现有 toast 的 `openDir` 惯例）。
- 术语人话：不出现 aria2/RPC/GID 等技术词；错误统一转译（同 `format.ts` 的 `errorText` 惯例）。

## 7. 安全与合规考量

| 议题 | 对策 |
|------|------|
| GPL 许可证传染 | aria2（GPLv2）/ Transmission（GPL-2.0/3.0 双授权，D2）只作为**独立进程**存在，AA4C 只通过网络协议（JSON-RPC / HTTP API）与之通信，不链接、不嵌入源码；随安装包分发时附带对应 LICENSE/NOTICE 文件，明确标注这是打包的第三方运行时依赖、许可证与 AA4C 自身代码（Apache-2.0）分开标注 |
| RPC 暴露面 | `rpc-listen-all` 保持关闭（只绑 127.0.0.1），`rpc-secret` 每次启动随机生成（Transmission 侧对应 `rpc-bind-address=127.0.0.1` + 随机 Basic 凭据，§3.6.2）；**密钥不走命令行参数**——命令行对本机任意用户的进程经 `ps`/WMI 可见，v1 的"拿不到密钥就调不了"在那个方案下不成立——改写进 data_dir 下 0600 权限的 conf 文件（§3.1）。诚实的边界声明：同一用户的其他进程本来就读得到 data_dir 里的一切（包括设备私钥），这不是本设计能扩大的边界；修掉的是"连**其他用户**的进程都能从进程列表里直接看到密钥"这个更大的洞 |
| 下载内容不受信 | 下载的是用户主动提供的任意公网 URL，AA4C 不做内容扫描/校验（同浏览器下载的信任模型，不是 AA4C 新引入的风险面）；aria2 能访问的网络范围与本机用户本身能访问的范围相同，不构成新的攻击面 |
| 下载目录默认隔离 | `download_dir` 是独立设置项，不自动并入任何同步范围/Inbox——避免用户下载的内容未经确认就被分享给已配对设备 |
| 子进程崩溃/被杀 | aria2c 异常退出时下载能力整体不可用，但不影响 AA4C 其余功能（同其余可选能力的一贯降级设计）；D1 先不做自动重启，观察实际故障率后再决定要不要加。反方向（AA4C 崩溃/被强杀、来不及跑 shutdown）由 `stop-with-process=<AA4C PID>` 兜底，aria2c 自行退出，不留孤儿进程（§3.1）。**Transmission 没有这个等价物**，异常路径的孤儿防护方案见 §3.6.2（Job Object / PDEATHSIG / PID 清扫组合，实现前小样验证） |
| 上游维护风险 | aria2 最新 release 是 1.37.0（2023-11），发版节奏明显放缓；Transmission 4.x 活跃维护中。对策：引擎版本由我们钉死并自建产物（§3.1），升级是显式受控动作、不自动跟随上游；RPC 只绑回环 + 密钥隔离，网络暴露面本就极小；`DownloadService` 对上层只暴露任务模型，引擎在背后可整体替换（D2 接 Transmission 本身就会验证这层抽象的可替换性） |
| Lua 插件（§10，预留） | 插件是**用户侧代码执行**，风险面与"下载不受信内容"不同量级——边界现在就写死：Lua 标准库的 io/os/加载器全部裁掉（默认零 IO）、宿主 API 走能力制（HTTP 访问按 manifest 声明的域名白名单、安装/启用时展示给用户确认）、不暴露文件系统与下载域之外的任何 Core 能力（设备身份/配对/同步一概不给）。实现推迟到独立里程碑并单独出安全评审，V0.4 期间任何人不得以"临时方便"为由先开小口子 |

## 8. 里程碑切分

1. **D1 ✅ — Aria2 集成（HTTP/HTTPS/FTP）**：sidecar 打包 + 生命周期管理（`SidecarSpawner` 注入点）+ `Aria2Client`（RPC + WebSocket 事件）+ `download_tasks` 表 + Core 集成（`CoreEvent`/Command）+「下载」页基础 UI（链接输入 + 任务列表 + 暂停/取消/打开文件夹）。已实现，见 §3.5 实现偏差。
2. **D2 ✅ — Transmission 集成（BT/Magnet）**（v3 换引擎，理由见 §3.6.1）：transmission-daemon sidecar（`-f` 前台模式）+ 孤儿进程防护（三平台均真实验证，§3.6.2）+ `TransmissionClient`（HTTP RPC + `X-Transmission-Session-Id` 握手，手写极简 HTTP/1.1 客户端）+ `download_tasks.kind='bt'` 分支（id=infohash）+ Core 编排（`add_download` 按 scheme 分流，两个引擎独立 actor）+ magnet 输入 UI + 做种数/peer 数/分享率展示（只进事件不落库）。已实现，见 §3.6.6 实现偏差。**唯一未完成**：正式打包分发管线（engines.yml transmission 构建腿 + `tauri.conf.json` externalBin），有意排到最后，同 D1 既定顺序原则。
3. **D3 — 统一任务中心打磨**：设置页新增「下载」区块（下载目录、并发数/限速/分享率透传给 aria2/Transmission 的 options）；D1+D2 任务在同一个列表里按时间统一排序；批量操作（全部暂停/清除已完成记录）。
4. **D4（预留，不在 V0.4 范围）— Lua 插件系统**：见 §10，动手前出独立设计文档 + 安全评审。

## 9. 已确认的设计细节

| 议题 | 决定 | 落点 |
|------|------|------|
| 下载引擎实现路径 | 不自研，包外部成熟工具（aria2 + BT 引擎，BT 选型见 v3 行），通过 RPC/API 调用、不链接源码——避免 GPL 传染，同时省下重新实现下载协议栈/BT 协议栈的巨大工作量 | §1，PROJECT_VISION.md §十三 |
| 子进程管理 | AA4C 自动打包 + 生命周期管理（Tauri sidecar），不要求用户自己装、自己配 RPC 地址 | §1.1，§3.1 |
| 首个里程碑范围 | 先做 Aria2（HTTP/HTTPS/FTP），BT/Magnet 后置为独立里程碑 D2（v1 时选型 qBittorrent，v3 换 Transmission）——两个外部依赖一起接入会让第一版的进程管理/错误处理/测试面同时翻倍 | §1.1，§8 |
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
| BT 引擎选型（v3） | qBittorrent → **Transmission**：三平台 headless 分发逐项核实后 qBittorrent 在 Windows/macOS 均无可用先例（Windows 完全空白），Transmission 官方 MSI 自带 daemon + Homebrew core formula 双架构齐全；RPC 更简单（header token vs cookie session）；无事件推送由 D1 已收敛的 `reconcile()` 轮询主路径补齐，不需要新机制 | §3.6 |
| BT 任务 id | infohash 直接当 `download_tasks.id`（引擎原生 id 原则的 BT 分支），跨重启天然稳定 | §3.6.4 |
| 做种策略（v3） | 下载完成标 `complete` 但**不停止做种**——保种是 BT 生态基本礼仪、私有 Tracker/PT（§10）的硬需求；分享率/做种时长限制 D3 透传设置 | §3.6.4 |
| Transmission 孤儿进程防护（v3，✅ 已小样验证） | Windows `CreateJobObjectW`+`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`+`AssignProcessToJobObject`；Linux `pre_exec` 内 `prctl(PR_SET_PDEATHSIG, SIGKILL)`；macOS PID 文件+进程身份（pid+启动时间+comm）核对后清扫。三者均用真实环境 PoC 验证通过（Windows/Linux 在真实 GitHub Actions runner 上，macOS 本机）——D2.4 直接照此实现 | §3.6.2 |
| 插件语言（v3 预留） | **Lua**（mlua vendored 编译进二进制，无系统依赖）：嵌入式脚本事实标准、运行时体量小、可彻底沙箱、非专业用户照模板可写；对比 JS 引擎（体量/复杂度高一个量级）、WASM（写作门槛过高）、Python（需带解释器分发）均不符合"简单 > 复杂" | §10 |
| 插件适用面（v3 预留） | 适用于**全部下载类型**，不是 BT 专属——HTTP 直链同样需要自定义请求头/鉴权/文件名规则/分类；钩子挂在引擎无关的任务模型层，不挂在具体引擎客户端上 | §10 |
| 插件权限模型（v3 预留） | 默认零 IO + 能力制宿主 API + manifest 域名白名单用户确认；V0.4 只预留接缝（引擎无关请求描述中间层），实现推迟到独立里程碑 + 独立安全评审 | §10，§7 |

## 10. Lua 插件系统（v3 预留设计——只定边界，不定 API）

### 10.1 动机

私有 Tracker/PT 站（登录态/passkey 注入、RSS 订阅、分享率与保种规则）、站点搜索/聚合搜索、任务自动分类（按规则决定保存子目录/打标签）、下载完成后的自动化动作——这类需求**高度站点化、长尾、变更频繁**，写死在 Rust 主程序里意味着每个站点的每次改版都要发一个应用版本，不现实；正确形态是用户/社区自己写脚本。且这不是 BT 专属：HTTP 直链下载同样需要自定义请求头/鉴权、文件名规则、分类。**自己做插件宿主，不依赖第三方插件生态。**

### 10.2 形态（预留稿）

- **宿主位置**：`PluginHost` 在 Core 层（纯 Rust，`mlua` vendored 编译进二进制——不引系统依赖、不破坏"Core 不依赖 Tauri、可被 Docker/headless 复用"的既定边界）。
- **钩子点**（初版枚举，精确签名留给实现期）：
  - `on_before_add(request) -> request`：改写下载请求（URL、请求头、引擎选项、保存子目录、分类）——PT 站 passkey 注入、HTTP 站点鉴权都在这里；
  - `on_task_complete(task)`：完成后的自动化动作（改名/移动到分类目录/通知）；
  - `search(query) -> [results]`：给统一搜索 UI 供数据（站内搜索/聚合搜索）；
  - `on_periodic()`：低频定时（RSS 拉取、保种策略巡检）。
- **权限模型**（§7 已写死，此处重述要点）：默认零 IO（裁掉 Lua 的 io/os/require）；HTTP 经宿主受控 API + manifest 域名白名单 + 用户启用时确认；不暴露文件系统；不暴露下载域之外的任何 Core 能力。
- **manifest 与分发**：`<data_dir>/plugins/<name>/`（manifest 声明名称/版本/钩子/域名权限），本地目录分发，无市场、无自动更新——插件市场的安全审查成本这个阶段不背。
- **对 D2/D3 的实际约束（现在就要做的只有这三条，其余全部推迟）**：
  1. 任务添加路径保留引擎无关的请求描述中间层（§5）——`on_before_add` 的天然挂点，D2 接第二个引擎本来就要做；
  2. `download_tasks` 的 `category` 列**不加**（迁移低成本，避免推测性设计，§4）；
  3. 事件总线既有，`on_task_complete`/`on_periodic` 挂点天然存在，无需预留动作。
- **里程碑归属**：V0.4 之外的独立里程碑（暂记 D4），动手前出独立设计文档，安全模型单独评审——插件是用户侧代码执行，风险量级不同于"下载不受信内容"。

## 仍待实现 / 后续

Windows MSI 解包产物的 DLL 伴随清单核实与 `bundle.resources` 打包实测（§3.6.5）；engines.yml transmission 构建腿首跑（预期按 aria2 首跑教训再踩一轮新坑）；限速/并发数/分享率等选项具体要透传 aria2/Transmission 的哪些 option、设置页字段怎么设计；下载失败的自动重试策略；子进程崩溃后的自动重启策略；"下载目录落在同步范围内"警示的具体交互（D3 设置页一起做）；`.torrent` 文件输入（D2 只接 magnet，文件输入留给 D3 或插件阶段）；引擎版本升级的操作流程文档化（engines release 的产出步骤，做进 CONTRIBUTING 或脚本注释）；Lua 插件系统的独立设计文档 + 安全评审（§10，D4）；Android 平台的下载能力方案（很可能是完全不同的技术路径，比如系统 DownloadManager，而不是 bundled 二进制，需要单独评估）；S3 协议支持（PROJECT_VISION.md 提到但未细化，大概率需要凭证管理，单独评估，不在 D1–D3 范围内）。
