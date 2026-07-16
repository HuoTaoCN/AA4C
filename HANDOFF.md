# AA4C 开发交接（换机指南）

> 最后更新：2026-07-16（**V0.4「Download」里程碑 D2（Transmission/BT-Magnet）已实现**：孤儿进程防护三平台真实验证通过、`TransmissionClient`/`DownloadService` 双引擎 actor/Core 编排/前端 UI 全部落地，真实进程集成测试覆盖路由/落库/暂停/继续/取消。**唯一没做完的是引擎二进制正式打包分发管线**（`engines.yml` transmission 构建腿 + `tauri.conf.json` externalBin），代码路径已就绪，sidecar 二进制暂缺时 BT 能力运行时优雅降级不可用，不影响已发布的 D1 功能。`engines.yml`（aria2）已首次真实跑通验证，`release.yml` 恢复 macOS universal 构建但还没在真实 tag push 里跑过。下一步是决策点：出新版本验证 D2 / 补引擎打包管线 / D3，见第四节）。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
> 给 AI Agent：开工前先读本文档"当前进度"与"下一步"，再按 [AGENTS.md](AGENTS.md) 的必读清单工作。

## 一、当前进度

| 里程碑 | 状态 | commit | 内容 |
|--------|------|--------|------|
| 文档体系 | ✅ | `9f5a2bc` | 16 份文档：愿景/架构/API/协议/数据库/UI/计划/测试/社区 |
| M0 工程脚手架 | ✅ | `15058a1` | Cargo workspace（6 crate）+ Tauri 2 + Vue3 + CI |
| M1 类型与存储 | ✅ | `06fe31e` | `aa4c-types` 全部类型；`aa4c-store` SQLite 迁移与 CRUD |
| —（文档）移动端改 Tauri 2 Android | ✅ | `c3dd037` | Flutter → Tauri 2 Android，新增 A0–A3 里程碑 |
| M2 设备身份 | ✅ | `62fab6d` | Ed25519 + TLS 1.3 mTLS 证书固定 + 配对 PIN |
| A0 Android 工程 | ✅ | `8a545b0` | gen/android 入库，本地与 CI APK 构建通过 |
| M3 设备发现 | ✅ | `af68939` | mDNS 广播/浏览/自身过滤/上下线事件，组播双实例测试本地通过 |
| M4 配对协议 | ✅ | `49dea79` | aa4c-proto 帧编解码 + PairingManager（双向 PIN/超时/写库），4 个 e2e 测试 |
| M5 传输引擎 | ✅ | `7fd6c1c` | 文件收发/BLAKE3 校验/路径净化/取消/断连/重传，8 个集成测试 + 1GB（ignored） |
| M6 Core + Tauri 桥 | ✅ | `fb726e5` | aa4c-core 组装五大组件 + 监听口分流配对/传输 + 11 个 Tauri Command + 事件转发，2 个端到端冒烟测试 |
| M7 前端 UI | ✅ | `4c94dbd` | Vue3+Router+Pinia 4 页面 + 配对/接收弹窗 + 任务条 + toast；拖拽/文件选择/通知；`pnpm build` 通过 |
| A1 Android 适配 | ✅ | `f955394` | MulticastLock + Manifest 权限 + 保存目录平台注入；aarch64 debug APK 本地构建通过；A2 响应式布局 M7 已含 |
| M8 / A3 发布 | ✅ | `e0557b7` | `v0.1.0` 已发布：三平台桌面包 + Android arm64 APK（CI tag 触发自动出包） |
| v0.1.1 联调修复 | ✅ | `9199f50` | 修代理(Clash fake-ip)环境下对端无法被发起配对的地址选择 bug + 默认设备名优化；已发 `v0.1.1` |
| V0.2 设计 | ✅ | `6d56ed5` | 新增 [SYNC_DESIGN.md](SYNC_DESIGN.md)：信任分级 + 跨设备文件索引 + 绿/黄/红状态可视化设计 |
| 产品重新定位 | ✅ | `8e02b27` | 品牌改「AA连接（AA4C）」、设备连接平台定位、连接优先五阶段路线；移动端确认沿用 Tauri 2 |
| 前端能力架构 + UI 预览 | ✅ | `2e78590`/`194286e`/`148e326` | 五大能力导航（传输/同步/分享/下载/归档）+ PC/移动两套外壳；同步页目录树预览 + 信任分级前移到配对成功弹窗 |
| V0.2 信任分级（第一步） | ✅ | `b12d739` | `devices.trust_level` 落库（迁移 `002_trust.sql`）+ `set_trust_level` 命令端到端打通，配对默认 friend，「我的设备/朋友」UI 接真实后端 |
| v0.2.0-preview 发布 | ✅ | `0ac48f6` | 打包预览版：品牌 + 新 UI + 信任分级随安装包/APK 发布（GitHub Release，prerelease） |
| V0.2 同步里程碑 2 | ✅ | `391b8b7` | 共享范围 + 本地索引扫描 + Inbox 落点（`003_sync.sql`，`aa4c-core/src/sync_index.rs`），「同步」页接真实本机文件 |
| V0.2 同步里程碑 3 | ✅ | `6962dca` | 跨设备索引摘要交换（`IndexRequest`/`IndexEntries`）+ `remote_index`（`004_remote_index.sql`）+ 统一视图绿/黄/红（`sync_exchange.rs` / `unified.rs` / `dispatch.rs`）；完全信任边界把关 + 降级清缓存，含 e2e 索引交换测试 |
| V0.2 同步里程碑 4 | ✅ | `7b35fe3` | 按需拉取（`FetchRequest` + `aa4c-transfer/src/fetch.rs` + `serve_fetch`）：点黄色「可下载」反转角色复用 ATP 拉内容→落 Inbox→扫描转绿；完全信任边界 + 只服务已索引条目，含 e2e 拉取测试 |
| V0.2 同步里程碑 5 | ✅ | `5e84031` | 冲突标记（同名不同 hash 加序号并列 + 分别拉取）+ `sync_conflicts`（`005_conflicts.sql`）；`unified::merge` 按 (path,hash) 拆版本、`UnifiedFile` 加 `basePath`/`conflict`，含 merge/store 单测 |
| 协议 proto→2 + 同步版本门槛 | ✅ | `d1d147a` | `PROTO_VERSION=2` + `SYNC_PROTO_VERSION`：索引/拉取发起方按协商版本 gate，遇 v1 对端优雅跳过；mDNS TXT `proto=2`；新增桌面联调钩子 `AA4C_DATA_DIR`/`AA4C_DEVICE_NAME`（同机跑多实例）+ `scripts/dev-two-nodes.sh`。**⚠️ 与 v0.2.0-preview 同步不再互通** |
| 同步收尾：实时监听 + 拉取落点镜像 | ✅ | `9dfe1ad` | `notify`（`notify-debouncer-mini`，2s 去抖）实时监听共享范围目录、随范围增删对齐（定时 300s + 传输完成兜底）；按需拉取按分组匹配落回本机对应范围原结构（原黄条目转绿），未命中回落 Inbox。含监听重扫测试 |
| v0.2.0-preview.2 发布 | ✅ | `bf403ba` | 打包预览版：V0.2 跨设备同步全链路 + proto=2 随三平台包/APK 发布（GitHub Release，prerelease）；`e53fb53` 修 CI `cargo audit`（忽略 quick-xml 传递依赖 DoS 公告） |
| V0.3 设计（Connect）v1 | ✅ | `df92c7b` | 新增 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)：连接阶梯 + 自建信令/中继 + QUIC + 远程能力复用 + 分享链接；DATABASE_SCHEMA §4c、PROTOCOL Part B、ROADMAP 同步 |
| V0.3 设计评审修订 + 实现计划 | ✅ | — | 设计定稿 v2：服务器身份=密钥对+地址内指纹、允许名单+挑战应答取代互签 proof、单进程 `aa4c-server`、信令复用帧层 bincode（弃 HTTP/WS）、单 `server_url` 默认关、分享仅已索引内容、**中继提前到打洞前**；新增 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)（C1–C6，C1 细化到可直接执行） |
| **V0.3 里程碑 C1（QUIC + 断点续传）** | ✅ | — | `aa4c-transfer/src/quic.rs`：QUIC 会话层（证书固定复用、单流等价迁移、keep-alive+8s 空闲超时）；`PROTO_VERSION=3` + `Message::ResumeReport`（追加变体）确定性断点续传（4 MiB 边界截断 + 重新流式喂哈希，不改 `Offer`）；只有明确取消才清理 `.aa4c-part`（顺带修了发送方内部取消不通知对端的既有小缺口）；`TransferConfig.prefer_quic` 测试开关；`IncomingIndexDispatch` 泛化到 `SharedStream`（TCP/QUIC 通用，配对仍限 TCP）；新增 e2e `quic_roundtrip_transfer` / `quic_resume_after_disconnect`（UDP 黑洞代理模拟真断连）；quinn 依赖与既有 rustls/ring 版本树验证对齐，`rust-version` 升 1.85 |
| **V0.3 里程碑 C2（`aa4c-server` 信令面）** | ✅ | — | 新 crate `crates/aa4c-server`（lib+bin）：身份复用 `aa4c-identity`，鉴权复用 mTLS（**未实现设计初稿 Challenge/ChallengeReply**，理由见 PROTOCOL.md §11）；`aa4c-proto::server` 新增独立 `ServerMessage` 族（`SrvHello(Ack)`/`Register`+`RegisterAck`/`Lookup`+`LookupReply`，帧层复用泛型化的 `read_message`/`write_message`）；注册表全内存态，覆盖式 `Register` 即吊销机制，TTL=60s（`REGISTER_TTL`）；`aa4c-core` 新模块 `server_link.rs`（客户端接入 + 后台续约循环）；`Settings` 新增 `server_url`/`enable_remote`（默认关）；`resolve_peer` 增加向自己服务器 Lookup 的第三档兜底（跨服务器好友寻址需要的 `devices.server_hint` 已建表但线路层交换留待后续，范围有意缩小）；交付含 Dockerfile + `scripts/dev-server.sh` + CI release Linux 二进制；新增 7 个确定性单测（`aa4c-server` 4 个 + `server_link` 3 个，不经 mDNS）+ 2 个 Core e2e 测试 |
| **V0.3 里程碑 C3（Relay 中继——远程可用自此成立）** | ✅ | — | `aa4c-server` 加中继面：`RelayRequest/Grant` 换一次性 token（8s TTL）+ `RelayOpen/OpenAck` 撮合后**裸字节透明转发**（对设计稿 `RelayData`/`RelayClose` 的收敛，不逐包重新编解码）；`RelayRequest` 不查允许名单，真正安全边界在被叫方自己的 `trusted` 检查（与直连同构）。`aa4c-core::server_link` 改用**一条常驻连接**周期续约 + 监听 `IncomingRelay` 推送，`Notify` 替代旧的一次性 `nudge_register` 连接立即生效——修了一个真实踩到的竞态（一次性连接与常驻连接抢推送登记槽位，断开时把常驻连接刚登记的活通道顶掉）。`aa4c-transfer` 新增 `RelayDialer` 注入 + `accept_external`（`dial()` 签名改 `Option<SocketAddr>`，直连失败/无地址时落中继）。新增 4 个 `aa4c-server` 单测 + Core e2e `forced_relay_path_completes_a_transfer`（强制走中继完成一次真实传输） |
| **V0.3 里程碑 C4（远程同步/发送接入连接阶梯 + 连接质量 UI）** | ✅ | — | `sync_exchange` 此前只认 mDNS 在线快照，改为遍历全部完全信任配对设备，与 `resolve_peer` 共用同一套地址解析阶梯（新增 `orchestrate::resolve_addr`）；`DeviceFound` 即时触发之外新增 30s 周期定时器兜底远程设备。`fetch_index`/`fetch_file` 的 `addr` 改 `Option<SocketAddr>`，同 `send()` 落中继兜底；`Core::fetch_file` 不再局限于 mDNS 在线持有者。在线判定并入"最近一次远程索引同步是否新鲜"（90s 窗口）。连接质量：新增 `ConnectionVia`/`CoreEvent::TransferConnected`，`dial()` 返回值带上实际档位，只有发起方收得到、只存内存不落库。前端：设置页新增「远程连接」区块（服务器地址 + 开关），传输卡片按 `via` 显示「直连/中继（较慢）」徽标。新增 Core e2e `remote_index_exchange_reaches_peer_via_relay` + `aa4c_types` 的事件 JSON 形状测试 |
| **V0.3 里程碑 C5（NAT 打洞）** | ✅ | — | `aa4c-server` 新增轻量 QUIC 反射端点（`reflect.rs`，与 TCP 信令同端口号，独立 ALPN）：设备用自己真正用于 P2P 的 QUIC 端点连一次，服务器把观测到的源地址经 uni 流回给它，自建版 STUN，不依赖公共 STUN 服务。`ServerMessage` 追加 `Signal`/`IncomingSignal`：发起方在自己的常驻连接上发候选，回信作为推送收回同一条连接（复用 C3 的 `pushable` 表）。`aa4c-core::server_link::SignalChannel` 处理候选交换；收到 `IncomingSignal` 时必须区分"是不是自己在等的回信"，不是才需要反向回信+打洞，否则死循环（真实踩到）。`aa4c-transfer` 新增 `PunchDialer`（`dial()` 第 3 档，直连失败后先试打洞再落中继）+ `ConnectionVia::Punch`（UI 上并入"直连"显示）。**顺带修了两个真实 bug**：(1) `QuicDuplex` 此前不持有 `Connection` 句柄，fire-and-forget 分流（`IndexRequest`）会在数据发出前就拆连接——从 C1 起潜伏，C5 才第一次踩中；(2) 回环环境打洞会稳定截胡"强制走中继"的测试，加了 `TransferConfig::disable_punch` 测试开关。新增 Core e2e `forced_punch_path_completes_a_transfer` + `aa4c-server` 反射端点/Signal 状态机单测 |
| **V0.3 里程碑 C6（分享链接 AA Share——V0.3「AA Connect」六个里程碑全部完成）** | ✅ | — | 新表 `shares`/`share_access`（`007_shares.sql`）；`Message` 追加 `ShareRequest{token}`（`PROTO_VERSION` 3→4）；`dispatch_shared` 的 `ShareRequest` 分支**不检查 `trusted`**——token 本身就是访问能力，未配对设备也能凭链接取回内容（对设计初稿的收敛，见 CONNECT_DESIGN.md §7.3）。`aa4c-transfer` 新增 `ShareResolver`/`TransferService::open_share`，`fetch.rs` 泛化出 `FetchTarget::{Path,Share}` 共用建连/握手/落盘。`aa4c-core::dispatch::ShareServe`（token 校验 + `resolve_shared` 路径边界 + `share_access` 记账）+ `orchestrate.rs` 的 `create_share`/`list_shares`/`revoke_share`/`open_share`。前端新增 `SharePage.vue`（生成/管理/打开）。**顺带修了一个真实 bug**：`transfer_tasks.peer_device_id` 的外键假设"peer 必然是已配对设备"被 `ShareRequest` 打破，插入任务记录会违反外键、把连接在协议中途悄悄挂断（现象是对端"connection lost"，真正原因在远端看不到）——`serve_fetch`/`fetch::drive` 现在先查一次对端是否已知，未知则跳过任务落库。新增 Core e2e `create_and_open_share_without_pairing`（从未配对的设备凭链接取回内容）+ `share_rejects_expired_revoked_and_forged_tokens` |
| v0.3.0-preview 发布 | ✅ | — | 打包预览版：V0.3「AA Connect」六个里程碑（QUIC+续传/自建信令/中继/远程同步发送+连接质量/NAT 打洞/分享链接）随三平台安装包 + Android arm64 APK + `aa4c-server` Linux 二进制发布（GitHub Release，prerelease）；工作区版本号 0.2.0→0.3.0 |
| V0.4 设计评审修订（v2） | ✅ | `c20f0dd` | 评审 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) 发现并修正 4 个实质问题：①官方 release 无 macOS/Linux 预编译产物（已逐项核实），改自建引擎构建流水线 + 校验和写死进仓库；②补齐 v1 缺失的任务跨重启恢复（aria2 `save-session` 管续传 + 启动对账管记录，普通 URI 下载 GID 跨重启不变是"GID=id"成立的前提）；③ `rpc-secret` 从命令行（`ps`/WMI 任意用户可见）改进 0600 conf 文件；④默认下载目录从自相矛盾的"save_dir 同级的 Downloads 子目录"改为系统下载目录——Inbox 索引根是整个 save_dir，落进子树=自动分享给完全信任设备。另补 `stop-with-process` 孤儿进程防护、端口竞态重试、进度写库节流、上游维护风险条目 |
| V0.4 实现计划 | ✅ | — | 新增 [V0.4_IMPLEMENTATION_PLAN.md](V0.4_IMPLEMENTATION_PLAN.md)：D1（Aria2 + 引擎流水线，细化到 10 步）→ D2（qBittorrent）→ D3（任务中心打磨）。关键排序决定：引擎二进制流水线放在代码**之后**——开发/测试/CI 全程用 PATH 安装的系统 aria2c（`ProcessSpawner` 本来就是 Docker/headless 要的实现），打包流水线零耦合、可单独攻坚不阻塞。设计 §3.2 同步收敛：RPC 载体从"HTTP+WS"改为 WS 单连接（少一个 HTTP 客户端依赖，id 关联表照 `SignalChannel` 先例） |
| **V0.4 里程碑 D1（Aria2 集成——下载中心第一次可用）** | ✅ | `d655c5c` | 新 crate `aa4c-download`（不依赖 Tauri）：`SidecarSpawner`/`EngineChild` 拆两个 trait（拉起 vs 终止，因为 Tauri `CommandChild::kill()` 同步、`tokio::process::Child::kill()` 异步，接口形状不同）；`conf.rs` 每次启动重写 aria2 配置文件（0600 权限，含随机端口/密钥/`stop-with-process`/`save-session`），命令行只传 `--conf-path`；`rpc.rs`（`Aria2Client`）JSON-RPC over WebSocket 单连接；`lib.rs`（`DownloadService`）单线程 actor 模型，公开方法经 channel 发命令；`reconcile()` 把"WS 通知""断线重连对账""轮询兜底"收敛成一段幂等逻辑。新表 `download_tasks`（`008_downloads.sql`）；`CoreEvent` 追加 `DownloadProgress`/`DownloadDone`/`DownloadFailed`；`Settings` 追加 `download_dir`；`Aa4cError` 追加 `Unavailable`。Tauri：`tauri-plugin-shell` + capabilities（`shell:allow-execute` + sidecar + 参数正则校验，**已通过真实 `tauri dev` 走查验证**）+ 5 个 Command；前端 `DownloadPage.vue`/`DownloadCard.vue`/`stores/download.ts`。CI/发布：`scripts/fetch-engines.sh`（`--from-path` 本地开发用系统 aria2c 顶位）+ `.github/workflows/engines.yml`（手动触发，未经真实 CI 验证）+ `ci.yml`/`release.yml` 相应改动；`tauri.conf.json` 新增 `bundle.externalBin`——**声明后任何触碰 `aa4c-desktop` 的 `cargo` 命令都要求该二进制文件存在，不是可选步骤**。**实测发现真实 aria2 行为**：`aria2.shutdown` RPC 内部等约 3 秒才退出，`SHUTDOWN_GRACE` 定为 5 秒。**人工走查中发现并修复真实 bug**：`aa4c-core` 下载端到端测试未隔离 `download_dir`，测试文件曾落进开发机真实 `~/Downloads`（已修复为 `Core::start` 前预置隔离路径到 settings 表）。新增 `aa4c-download` 6 条真实 aria2c 集成测试 + 4 条 conf 单测；`aa4c-store`/`aa4c-types`/`aa4c-core` 各自补充测试 |
| v0.4.0-preview 发布 | ✅ | — | 打包预览版：V0.4「Download」里程碑 D1（下载中心，HTTP/HTTPS/FTP 直链）随三平台安装包 + Android arm64 APK 发出（GitHub Release，prerelease）；工作区版本号 0.3.0→0.4.0。**macOS 这次只出 arm64 安装包**——`engines.yml`（引擎自建流水线）还没真正跑过、校验和是空的，`release.yml` 临时改用 `--from-path`（CI runner 装的包管理器版本 aria2c）打包，而 arm64 runner 上拿不到 x86_64 版本，Intel Mac 用户暂时用不了这个 preview；`release.yml` 的 macOS 构建目标临时从 `universal-apple-darwin` 改成 `aarch64-apple-darwin`，两处都留了 `TODO(engines.yml 验证后恢复)` 注释。协议完全兼容（`PROTO_VERSION` 未变，D1 不碰线路协议），无需强制同步升级。**⚠️ 事后发现：这个版本的 Windows 安装包打包了一个不能用的 aria2c.exe**（`choco install aria2` 装的是 chocolatey 的 shim，不是真正的可执行文件——细节见第五节教训），下载功能在 Windows 上运行时会失效；已在 `v0.4.0-preview.1` 修复重发，见下一行 |
| v0.4.0-preview.1 发布（Windows 安装包修复重发） | ✅ | — | 用户决定切一个新 tag（而非移动已发布的 `v0.4.0-preview`）来修复上一行的 Windows 问题。`release.yml`/`ci.yml` 不再用 `choco install aria2`，改成直接下载官方 aria2 Windows release 压缩包解出真正的二进制。**这个版本除 Windows 安装包外没有任何功能或代码变化**——`aa4c-download`/`aa4c-core`/前端与 `v0.4.0-preview` 完全一致，macOS/Linux 安装包未重新发布（choco 只用在 Windows，不受影响，仍是同一份 arm64-only macOS 产物）。发布前跑了完整本地校验：`cargo fmt --check`/`cargo clippy -D warnings` 干净；`cargo test --workspace` 单线程下 15/15 `aa4c-core` 测试全过，含 `download_end_to_end_through_core_orchestration`；并行下 `quic_resume_after_disconnect`/`two_cores_pair_then_transfer` 两条复现了既有的并行环境 flaky（HANDOFF.md 早有记录，非本次改动引入）；前端 `vue-tsc --noEmit && vite build` 通过 |
| `engines.yml` 首次真实跑通 | ✅ | `7a45d9a` | 手动触发 `workflow_dispatch`（`aria2_version=1.37.0`），四轮才全绿，过程踩了四个此前未知的坑（macOS 交叉编译链接失败、GitHub `macos-13` 镜像已退役、Ubuntu `autopoint`/`liblzma-dev` 缺包、`libc-ares-dev` 无静态库改 `--without-libcares`），详见第五节教训。产物 + `SHA256SUMS` 发到 `engines/aria2-1.37.0` release，校验和填进 `scripts/fetch-engines.sh`；`release.yml` 恢复正式状态（macOS `universal-apple-darwin`，三平台真实校验和下载，不再依赖系统包管理器）——**这条改动本身没有触发新的应用发布**，只影响下次真实 tag push 时 release.yml 的行为，还没在真实 release 里跑过一次 |
| V0.4 设计 v3（D2 换引擎 + Lua 插件预留） | ✅ | `37d4906` | 两个决定：① **D2 的 BT 引擎从 qBittorrent 换成 Transmission**——按 D1 教训逐项核实三平台 headless 分发后，qBittorrent 在 Windows 官方/社区完全空白、macOS 官方缺失且上游有争议；Transmission 官方 Windows MSI 自带 daemon、macOS/Linux 有 Homebrew core 验证过的 CMake 配置可复用（新增 DOWNLOAD_DESIGN.md §3.6 完整设计）。② **预留 Lua 插件系统设计边界**（新增 §10）：私有 Tracker/PT/搜索/自动分类等长尾需求走用户可写 Lua 插件，权限模型（默认零 IO + 能力制 + 域名白名单）现在写死，实现是独立里程碑，D2 只需保留引擎无关请求描述中间层这一个接缝 |
| **V0.4 里程碑 D2（Transmission 集成——BT/Magnet 下载可用）** | ✅ | `4a7fea1`+`752fbac`+当前 | 孤儿进程防护三条路径（Windows Job Object / Linux `PR_SET_PDEATHSIG` / macOS PID 文件）**全部真实环境 PoC 验证通过**（不是纸面设计，见第五节教训）；`transmission_conf.rs`（settings.json 生成）+ `transmission_process.rs`（spawn 生命周期）+ `transmission_rpc.rs`（`TransmissionClient`，手写 HTTP/1.1 客户端处理 409 CSRF 握手）。`SidecarSpawner::spawn` 签名从单参数泛化成 `args: &[String]`（Transmission 命令行是 `-f --config-dir=X` 两参数，D1 原签名放不下）；`DownloadService` 内部变成两个独立 actor，`add()` 按 scheme 分流、`pause`/`resume`/`cancel` 按任务 id 长度分流（aria2 GID 16 位 vs BT infohash 40 位十六进制，协议本身决定的固定长度）；两引擎可用性完全独立。`CoreEvent::DownloadProgress` 追加 `seeders`/`peers`/`ratio` 可选字段，复用同一事件不新开变体。`CoreConfig` 新增 `bt_spawner`，桌面壳层注入 `transmission-daemon` sidecar；capabilities.json 加对应 `shell:allow-execute` 权限。新增真实进程集成测试：`aa4c-download` 单测+`tests/transmission.rs`（session 握手/401/torrent-add-remove）、`aa4c-core` 的 `bt_download_routes_through_core_orchestration`（真实双引擎通过 Core 编排验证路由/落库/暂停/继续/取消）。前端：magnet 输入、`taskTitle()` 解析 `dn=` 显示名、`DownloadCard.vue` 展示做种数/连接数/分享率。**未完成**：引擎二进制正式打包分发管线（`engines.yml` transmission 构建腿 + `tauri.conf.json` externalBin），有意排到最后，见 DOWNLOAD_DESIGN.md §3.6.6 与第四节 |

整个 V0.1 桌面端链路 **发现 → 配对 → 传输 → UI** 已全部打通。**V0.3「AA Connect」六个里程碑（C1–C6）全部完成**：广域网 QUIC 会话层、自建信令+中继服务器、远程同步/发送接入完整连接阶梯、NAT 打洞、分享链接，一整条「局域网直连 → 公网直连 → 打洞 → 中继」的连接阶梯贯通，外加脱离设备配对关系的能力型分享。**V0.4「Download」里程碑 D1（Aria2/HTTP-FTP）+ D2（Transmission/BT-Magnet）均已实现**，已打包发布 `v0.4.0-preview.1`（只含 D1；D2 是这之后新做的，还没出新版本）——新 crate `aa4c-download` 同时管两个引擎、下载页支持直链+magnet、真实 `tauri dev` 走查跑通（sidecar 拉起、Tauri capability 权限、孤儿进程防护三平台均实测有效）；D3（任务中心打磨）仍是设计稿。**D2 唯一没做完的是引擎二进制正式打包分发管线**（`engines.yml` transmission 构建腿 + `tauri.conf.json` externalBin），代码路径完全就绪，只是 sidecar 二进制实际不存在时 BT 能力在运行时优雅降级不可用。`engines.yml` 已首次真实跑通验证（详见上表），`release.yml` 恢复 macOS universal 构建，但还没在真实 tag push 里验证过。**V0.2 同步五个里程碑（信任分级 / 本地索引 + Inbox / 跨设备索引交换 + 统一视图 / 按需拉取 / 冲突标记）全部落地**（SYNC_DESIGN.md §10）；线路协议已升到 `proto=2` 并对同步路径按版本 gate（与 v0.2.0-preview 的同步不再互通，趁预发布窗口对齐）。**真机 GUI 走查已人工跑通**（`scripts/dev-two-nodes.sh` 起两实例：配对 → 互标我的设备 → 黄「可下载」→ 点黄拉取转绿 → 同名不同内容「多版本」并列，均正常）。

### 已实现 crate 概览（`crates/`）

| crate | 职责 | 关键公共 API |
|-------|------|--------------|
| `aa4c-types` | 公共类型 | `DeviceInfo` `TransferTask` `CoreEvent` `Aa4cError`（含 `code()`）；常量 `DEFAULT_PORT=42420` `CHUNK_SIZE` `MAX_FRAME_LEN` |
| `aa4c-proto` | 线路协议 | `Message` 枚举、`read_message`/`write_message`/`encode_frame`、`client_hello`/`server_hello` |
| `aa4c-identity` | 身份 + 配对 | `Identity::load_or_generate`、`tls_server_config`/`tls_client_config`（mTLS 证书固定）、`derive_pin`、`PairingManager`（`start_pairing`/`handle_incoming`/`confirm`） |
| `aa4c-discovery` | mDNS | `DiscoveryService::new/start/stop/devices` |
| `aa4c-store` | SQLite | `Store::open`、设备/任务/设置/分享 CRUD（`Store` 是廉价克隆句柄，内部专职线程）；`insert_share`/`list_shares`/`get_share_by_token`/`revoke_share`/`record_share_access`/`list_share_access`（`shares`/`share_access` 表，里程碑 C6） |
| `aa4c-transfer` | 传输 + 索引交换 + 按需拉取 + QUIC + 中继 + 打洞 + 分享 | `TransferService::new`（返回 `Arc<Self>`）、`start_listener`/`send`/`accept`/`cancel`/`fetch_index`/`fetch_file`/`open_share`/`accept_external`/`reflexive_addr`/`punch_probe`；`set_pair_dispatch` / `set_index_dispatch` / `set_fetch_resolver` / `set_share_resolver` / `set_relay_dialer` / `set_punch_dialer` 注入钩子（`IncomingPairDispatch` / `IncomingIndexDispatch` / `SharedFileResolver` / `ShareResolver` / `RelayDialer` / `PunchDialer` trait）；推送与拉取共用 `recv::receive_files` + `send::serve_fetch`（`serve_fetch`/`fetch::drive` 里的任务落库现在会先判断对端是否已知设备，见 C6 教训）；`quic.rs` 会话层（`QuicDuplex` 持有 `Connection` 句柄，见 C5 教训）；`dial()`（`pub(crate)`）直连失败依次尝试打洞、中继；`TransferConfig::disable_punch`/`prefer_quic` 是测试专用开关 |
| `aa4c-core` | 组装 | `Core::start`/`shutdown`/`subscribe`/`self_info`/`listen_port`；§9 的 11 个 Command 在 Core 上有同名编排方法；`CoreConfig`、`Settings` 读写；`server_link.rs`（自建服务器客户端接入：一次性 `register_once`/`lookup_once` + 常驻连接 `spawn_register_loop`，返回 `(Notify, Arc<SignalChannel>)`——`Notify` 供 `nudge_register` 立即唤醒重新注册，`SignalChannel` 供 `PunchDialerImpl` 提交打洞候选请求）；`orchestrate.rs` 新增 `create_share`/`list_shares`/`revoke_share`/`list_share_access`/`open_share`（里程碑 C6）；`dispatch::ShareServe` 实现 `ShareResolver` |
| `aa4c-server` | 自建信令 + 中继 + 打洞反射服务器（bin+lib） | `run(ServerConfig{data_dir, listen_addr}) -> Arc<Server>`；`Server::device_id`/`local_addr`/`address_with_host`；内嵌 `run()` 供测试驱动，供部署用 `main.rs`（`AA4C_SERVER_DATA_DIR`/`AA4C_SERVER_LISTEN` 环境变量）；中继面（`RelayRequest`/`RelayOpen` 等）与打洞面（`Signal`/`IncomingSignal`）都随常驻连接的 `Register` 一并处理，无独立公开 API；`reflect.rs` 额外绑定一个轻量 QUIC 反射端点（同端口号，独立 ALPN） |
| `aa4c-download` | 下载中心（V0.4 里程碑 D1 aria2 + D2 Transmission，不依赖 Tauri） | `DownloadService::start(spawner, bt_spawner, store, events, data_dir, download_dir) -> Arc<Self>`（内部两个独立单线程 actor，aria2 一个、Transmission 一个，可用性互不影响）；`add`/`pause`/`resume`/`cancel`/`list`/`shutdown`（`add` 按 URL scheme 分流，`pause`/`resume`/`cancel` 按任务 id 长度分流：aria2 GID 16 位 vs BT infohash 40 位十六进制）；`SidecarSpawner`/`EngineChild` trait（`spawn(args: &[String])`，`ProcessSpawner` 是 Docker/headless 与测试共用的实现，Tauri 壳层的 `TauriSidecarSpawner` 见 `apps/desktop/src-tauri/src/download_spawner.rs`，构造时传 sidecar 名字）；`Aria2Client`（JSON-RPC over WebSocket 单连接）；`TransmissionClient`（手写 HTTP/1.1，`X-Transmission-Session-Id` 握手）；`TransmissionProcess`（spawn 生命周期 + 孤儿进程防护）；`orphan_guard`（Windows Job Object / Linux `PR_SET_PDEATHSIG` / macOS PID 文件，三平台真实验证） |

CI 现状：7 个 job 全绿（lint、三平台 test、frontend、audit、android 哨兵）；三平台 lint/test job 新增 aria2 安装 + `scripts/fetch-engines.sh --from-path` 步骤（V0.4 D1 起 `aa4c-desktop` 的 `externalBin` 声明要求该二进制存在才能编译）。

## 二、新电脑环境安装（macOS）

### 必装（桌面轨开发，约 10 分钟）

```bash
# 1. Homebrew（如未装）https://brew.sh
# 2. Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# 3. Node 工具链 + aria2（下载中心 sidecar，V0.4 里程碑 D1 起必装，见下方说明）
brew install node pnpm gh aria2
# 4. gh 登录
gh auth login
# 5. 克隆并验证
git clone https://github.com/HuoTaoCN/AA4C.git && cd AA4C
bash scripts/fetch-engines.sh --from-path   # 见下方「下载引擎二进制」说明，缺这步 cargo build 会报错
cargo test --workspace            # Rust 全绿
cd apps/desktop && pnpm install && pnpm tauri dev   # 应出现 AA4C 欢迎窗口
```

**下载引擎二进制（V0.4 里程碑 D1 起必做，一次性）**：`apps/desktop/src-tauri/tauri.conf.json` 声明了
`bundle.externalBin`（Tauri sidecar 机制，供 `aa4c-download` 拉起 aria2c），一旦声明，
`tauri_build::build()` 会在**任何** `cargo build`/`cargo check`/`cargo test`/`cargo clippy`
碰到 `aa4c-desktop` 这个 crate 时校验对应二进制文件存在——不是可选步骤，也不只影响下载相关代码，
整个工作区的 Rust 命令都会失败，直到你跑过一次
`bash scripts/fetch-engines.sh --from-path`（把 PATH 里刚装的系统 aria2c 复制到
`apps/desktop/src-tauri/binaries/` 顶位，不校验、仅本地开发用，产物已被 `.gitignore` 排除）。
真正发版用的引擎二进制走 `.github/workflows/engines.yml`（手动触发，按写死校验和下载，见
DOWNLOAD_DESIGN.md §3.1）。

### 可选（Android 轨，A1 起需要，约 30-60 分钟）

```bash
# JDK 17
brew install openjdk@17
export JAVA_HOME=/opt/homebrew/opt/openjdk@17

# Android cmdline-tools（brew cask 经代理易断，建议手动下载）
mkdir -p ~/Library/Android/sdk/cmdline-tools
curl -L -C - --retry 8 -o /tmp/cmdtools.zip \
  https://dl.google.com/android/repository/commandlinetools-mac-14742923_latest.zip
cd /tmp && unzip -q cmdtools.zip && mv cmdline-tools ~/Library/Android/sdk/cmdline-tools/latest

# SDK 组件（版本必须与 CI 一致）
export ANDROID_HOME=~/Library/Android/sdk
SDKM="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
yes | "$SDKM" --licenses
"$SDKM" "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;27.1.12297006"

# Rust Android targets
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

# 验证：构建 debug APK
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
cd AA4C/apps/desktop && pnpm tauri android build --apk --target aarch64 --debug
```

建议把这几个 export 写进 `~/.zshrc`：`JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME`。

## 三、注意事项（这台机器上踩过的坑）

### 网络 / 代理（最大坑源）

环境是 Clash 类代理 `127.0.0.1:7897`（HTTP_PROXY/HTTPS_PROXY 环境变量）：

1. **cargo 必须关 HTTP 多路复用**，否则经代理拉 crates 报 "HTTP2 framing layer" 错误：
   ```bash
   printf '[http]\nmultiplexing = false\n' >> ~/.cargo/config.toml
   ```
2. **git push**：代理开着时走代理即可；若报 "Failed to connect to 127.0.0.1:7897" 说明代理没开，临时 `git -c http.proxy= push` 直连（但代理开着时直连 GitHub 反而超时）
3. **大文件下载经代理常被截断**（curl 18 partial file）：brew bottle、NDK、Gradle 都中过招。对策：`curl -C - --retry 8` 断点续传循环；Gradle 发行包直接用腾讯镜像不走代理：
   ```bash
   curl --noproxy '*' -L -O https://mirrors.cloud.tencent.com/gradle/gradle-8.14.3-bin.zip
   # 放入 ~/.gradle/wrapper/dists/gradle-8.14.3-bin/<hash>/ 目录
   ```
4. push 偶尔报 "the remote end hung up unexpectedly" 但实际已成功——用 `git ls-remote origin` 核实再决定是否重推

### 工程约定

5. **pnpm 11**：构建脚本许可在 `apps/desktop/pnpm-workspace.yaml` 的 `allowBuilds`（不是 package.json 的 pnpm 字段）
6. **gen/android 工程源文件已入库**；Gradle 产物/schemas 由各级 .gitignore 排除，不要把 `app/build/`、`.so`、APK 提交进来
7. **接口变更先改文档**：API_DESIGN / PROTOCOL / DATABASE_SCHEMA 是唯一事实来源
8. 提交前自检：`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
9. CI 的 android job 是 `continue-on-error` 哨兵，不阻塞合并；其余 job 必须全绿。`cargo audit` 忽略了 `RUSTSEC-2026-0194/0195`（quick-xml DoS，Tauri 打包链路传递依赖、运行时不碰不可信 XML，无法就地升级），上游 Tauri/plist 升到 quick-xml ≥0.41 后应移除该忽略（见 `ci.yml` 注释）
10. `gh run watch` 经代理不稳定（annotations 接口 EOF），盯 CI 用轮询：
    ```bash
    gh api repos/HuoTaoCN/AA4C/actions/runs/<id>/jobs --jq '.jobs[] | "\(.name): \(.conclusion // .status)"'
    ```

## 四、下一步：V0.4 里程碑 D1+D2 均已实现，等用户选下一步方向

**V0.3「AA Connect」六个里程碑（C1–C6）全部实现完毕并测试通过，且已打包发布 `v0.3.0-preview`**：连接阶梯「局域网直连 → 公网直连 → 打洞 → 中继」四档贯通，外加脱离配对关系的能力型分享，随三平台安装包 + Android arm64 APK + `aa4c-server` Linux 二进制一起发出（GitHub Release，prerelease）。设计见 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)（§12 已确认决策清单）、实现拆解见 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)。

**V0.4「Download」里程碑 D1（Aria2，HTTP/HTTPS/FTP）已实现并发布 `v0.4.0-preview.1`；里程碑 D2（Transmission，BT/Magnet）代码已实现但还没出新版本发布**：D1 新 crate `aa4c-download`（不依赖 Tauri）+ `SidecarSpawner` 依赖倒置；D2 在同一个 crate 里加了第二个独立 actor（Transmission），`DownloadService` 现在统一管两个引擎，`add()` 按 URL scheme 分流、`pause`/`resume`/`cancel` 按任务 id 长度分流。孤儿进程防护三平台（Windows Job Object / Linux `PR_SET_PDEATHSIG` / macOS PID 文件）**全部真实环境 PoC 验证过**，`bt_download_routes_through_core_orchestration` 用真实双引擎通过 Core 编排验证了路由/落库/暂停/继续/取消全部生效（细节见 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) §3.5「D1 实现偏差」/ §3.6.6「D2 实现偏差」）。「下载」页支持 HTTP 直链与 magnet 链接、暂停/继续/取消/打开文件夹，BT 任务额外展示做种数/连接数/分享率。**受限于本环境没有原生 GUI 自动化工具，没有做真机 `tauri dev` 点击走查**（D1 当初做过一次，D2 这轮只验证到真实进程集成测试这一层）——如果你本人在桌面上粘贴一条 magnet 链接走一遍加/暂停/继续/取消，能补上这最后一段信心。

**⚠️ 待决问题——D2 的引擎二进制正式打包分发管线还没做**：`engines.yml` 缺一条 Transmission 构建腿，`tauri.conf.json` 没有为 `transmission-daemon` 声明 `bundle.externalBin`（`aria2c` 有）——这意味着**即使代码全部就绪，打出来的桌面安装包里现在也不会真的带上 transmission-daemon 二进制**，用户点击 magnet 链接会在运行时收到"BT 能力不可用"（优雅降级，不会崩溃，但功能形同虚设）。`ci.yml` 已经装了 transmission-daemon（含验证过可行的 Windows 官方 MSI 静默解包提取方法）但那只服务于跑测试，不等于发布管线也有了。这是照抄 D1 的既定顺序原则（"引擎二进制流水线排在代码之后"）刻意留到现在的——要不要现在就动手把这条管线补上（参考 aria2 那次 engines.yml 首跑踩了四个坑的经验，Transmission 大概率也要踩新的坑），还是先出一版"D2 功能已实现但 BT 暂时不可用"的预览版验证代码路径，这个决定交给用户。

**其他待决问题**：`v0.4.0-preview.1` 之后 main 上已经有相当规模的新代码（D2 全部实现 + `engines.yml` 首跑修复 + `release.yml` 恢复正式状态但没在真实 tag push 里验证过）——要不要现在切一版新预览（`v0.4.0-preview.2` 或等 D2 打包管线补完再一起发），也是需要用户决定的事，不要自己假设。

**D2 遗留的下一步选项**（不要凭本文档自己假设该做哪一个，直接问用户）：
- **补 D2 引擎打包分发管线**（见上，`engines.yml` transmission 构建腿 + `tauri.conf.json` externalBin）——不做完这个，D2 对真实用户来说等于没有。
- **D3（任务中心打磨）**：设置页下载区块（目录/限速/并发/分享率）、下载目录落在同步范围内的警示交互、批量操作、D1+D2 任务统一排序（数据模型上已经是同一张表，UI 排序目前也已经是按 `created_at` 统一的，D3 主要是打磨而非从零搭）。
- 或补 V0.3 已知缺口（见下）——都不冲突，可穿插进行。

**V0.3 范围内已知的、有意缩小的缺口**（不阻塞任何已完成里程碑，可随时单独补）：
- `devices.server_hint` 已建表但配对协议未交换它，`resolve_peer`/`sync_exchange`/中继的 `RelayDialer`/打洞的 `PunchDialer`/分享的地址解析目前都只查/连**自己配置的服务器**——跨服务器好友寻址（含跨服务器分享）还不可用，只覆盖「自己的多台设备」+「双方恰好用同一服务器」两种场景；交换 server_hint 需要一条新的追加协议消息（`PairRequest`/`PairAccept`/`DeviceInfo` 是既有结构体，不能直接加字段）。
- 分享链接：`aa4c://` deep-link 系统级注册（桌面三平台 + Android intent）未做，首版只支持粘贴链接打开；二维码生成未做。
- 打洞的真实穿透成功率需要人工双网络验证（回环/CI 只能验证候选交换+连接接线本身是对的）；单任务多流优化（每文件独立流、并行与独立重传）视情况以后再做，目前仍是首版单流等价迁移。
- keep-alive 目前用固定 8s 空闲超时+2s 心跳（已验证够用）；按需拉取（fetch）路径暂不支持续传（仅 Offer/send 路径支持）。
- **可随时补的 V0.2 尾巴**（不阻塞 V0.3）：Inbox 按来源设备+时间分组、`IndexSummary` 摘要优化、冲突版本历史 / 自动合并。

> ⚠️ 版本兼容：proto 现为 4（C6 起，`Message::ShareRequest`）；与更旧对端握手自动协商降级，行为不变（不发送对方不认识的高版本消息）。`v0.2.0-preview.2` 起的构建可与本版本互通同步；与 v0.1.x 仍因 `DeviceInfo.trust_level` 无法配对。
>
> 本机联调服务器：`bash scripts/dev-server.sh`（启动日志会打印 `aa4c://<host>:<port>#<指纹>`，把 `<host>` 换成客户端能连到的地址，填进设置的 `server_url` + 打开 `enable_remote`）。

### A1 已完成要点（Android 适配）

- `MainActivity` 持有/释放 `WifiManager.MulticastLock`（mDNS 组播必需）
- Manifest 加 `ACCESS_NETWORK_STATE` / `CHANGE_WIFI_MULTICAST_STATE` / `POST_NOTIFICATIONS`
- 接收目录由 Tauri path resolver 注入（桌面=下载目录，Android 回落应用目录），见 `src-tauri/src/lib.rs` setup 与 `aa4c-core` 的 `save_dir_fallback`
- aarch64 debug APK 已本地构建通过，产物在 `gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`（约 194 MB，含调试符号）

### Android 构建环境坑（务必记下）

- `gen/android/app/build.gradle.kts` 用 `compileSdk=36`（androidx lifecycle 2.10/webkit 1.14 要求），**必须装 `platforms;android-36`**；AGP 默认 buildTools 为 **35.0.0**，也要装：
  ```bash
  sdkmanager "platforms;android-36" "build-tools;35.0.0" "build-tools;36.0.0"
  ```
- 本地 Gradle 自动安装 SDK 组件会报 "SDK directory is not writable"（已知怪癖，目录其实可写）——**用 sdkmanager 手动预装**即可绕过
- 经代理下载 SDK 包常在 33% 截断（`Error reading Zip content`）——**删除半成品目录重试**，一般 1–2 次成功
- 构建：`cd apps/desktop && pnpm tauri android build --apk --target aarch64 --debug`（先 `export JAVA_HOME/ANDROID_HOME/NDK_HOME`）
- CI android 哨兵已对齐 `platforms;android-36` + `build-tools;35.0.0`

### 前端自测要点（联调时用）

- **同机双实例联调**：`bash scripts/dev-two-nodes.sh` 一键构建并起 A / B 两个隔离窗口
  （靠后端钩子 `AA4C_DATA_DIR` / `AA4C_DEVICE_NAME`，见 `src-tauri/src/lib.rs` setup），
  脚本头部有完整走查清单（配对 → 互标我的设备 → 黄 → 拉取转绿 → 冲突多版本）。
- 单实例快速起：`pnpm tauri dev`。
- ✅ 同步链路已人工走查通过（2026-07-03，`scripts/dev-two-nodes.sh` 两实例）。注意 GUI 走查
  只能人工做：自动化（computer-use）的截图过滤只认 LaunchServices 注册的桌面 app，CLI 起的
  当前构建实例对其不可见，加上双向配对要两窗口同时点——后端逻辑另有真 TLS 端到端测试兜底
  （`index_exchange_gated_by_full_trust` / `on_demand_fetch_...`）。
- 前端代码在 `apps/desktop/src/`：`lib/`（api/events/format/types）、`stores/`（Pinia）、`pages/`、`components/`。

## 五、本次会话的教训（务必遵守）

- **一次只跑一个后台测试任务**：之前同时挂多个重叠的 `cargo test` 后台任务，被 harness 标记 killed 后留下僵尸测试二进制（占 TCP 端口 / Store 线程），导致后续测试互相抢占、越来越慢甚至卡死。跑测试前先 `ps -eo pid,etime,command | grep aa4c_` 确认无残留。
- **`cargo test --workspace` 会跨 crate 并行跑测试二进制**，单独 `cargo test -p X` 过不代表 workspace 过。提交前务必跑一次完整 `cargo test --workspace`。
- **lib 内联单测 ≠ 集成测试**：`cargo test -p X --test Y` 只跑集成测试，漏掉 `src/*.rs` 里的 `#[cfg(test)]`。要 `--lib` 或直接 `--workspace` 覆盖全部。
- **`crates/aa4c-core/tests/core.rs` 在多核开发机上默认并行跑会偶发抖动**（本机 10 核）：多个测试各自起 2-3 个真实 Core（真 mDNS + 真 TCP/QUIC），默认测试线程数=核数时会出现 `No route to host`（真实回环连接被拒，非代码 bug）、mDNS 命中到不可直接拨号的 IPv6 link-local 地址（`fe80::...`，缺 zone id）等偶发失败——**已确认与本会话新增的 `resolve_peer`/`server_link` 代码无关**：诱因是老测试 `two_cores_pair_then_transfer`/`quic_roundtrip_transfer`（C1 及更早）在同样的高并发下也会失败，且用 `--test-threads=1`（或 2）时全部 8 个测试稳定全过。CI 跑在核数较少的 runner 上大概率不受影响；本机复现/复查用 `cargo test -p aa4c-core --test core -- --test-threads=1`。这是环境特性，不是本里程碑要修的 bug。
- **一次性短连接与常驻连接抢同一个"推送登记槽位"是真实竞态，不是理论风险**（C3 教训）：最初给中继加"被叫方能收到服务器推送"这个能力时，让 `nudge_register`（设置变更/解除配对触发的一次性 `Register`）和新增的常驻连接**都**去登记 `pushable[device_id]`，想着"只在已登记通道已关闭时才覆盖"就够安全——实测发现一次性连接的 `Register` 发送时刻，常驻连接的通道往往还没来得及登记（或反过来），加上一次性连接发完就断开、断开时的清理会把刚登记好的活通道顶掉，导致接下来一段时间（最长 TTL/3）中继推送悄悄收不到。**排查方法**：给协议关键路径临时加 `eprintln!`（`tracing` 在 `#[tokio::test]` 里默认没有订阅者，看不到任何输出，必须手动打印或临时接一个 subscriber），跑单测试 vs 跑整个测试文件对比日志，能看到"注册了，但紧接着被顶掉"的确切时序。**根治方案**：不要试图用条件判断在两个竞争的注册源之间做仲裁，而是从设计上消除第二个注册源——用 `tokio::sync::Notify` 唤醒**唯一**的常驻连接立刻重新注册，不再让任何一次性连接碰这条状态。同理，测试里如果某个操作理论上应该"立即生效"，不要想当然认为它真的是同步/瞬时的——`enable_remote` 打开后到常驻连接真正完成握手注册之间有真实的网络往返耗时，测试/生产代码都不能假设为零。

- **`sync_exchange`/`resolve_peer` 各自维护一份地址解析逻辑，容易跑偏**（C4 教训）：C2/C3 只给 `send_files`（`resolve_peer`）接了连接阶梯，`sync_exchange`（远程索引同步）当时图省事继续用 `discovery.devices()`（纯 mDNS），当时看起来是"先跑通直连再说"的合理取舍，但拖到 C4 才发现这其实是两套并行维护的解析逻辑——任何一处后续改动（比如再加一档）都得同步改两个地方，很容易漏改。**教训**：一旦发现"这段逻辑我在另一个模块写过一遍"，当场抽成共享函数（本例是 `orchestrate::resolve_addr`），不要等到"以后再补"——往后拖的每一个新调用点都是新的技术债，且下一次读代码的人（包括未来的自己）会默认"这两处都是权威实现"而各自改各的。
- **mDNS 的 `DeviceFound` 不是"设备上线"的完整信号，只是"局域网设备上线"**（C4 教训）：`sync_exchange` 原来完全靠 `DeviceFound` 触发索引刷新，这个假设对局域网设备成立，但对纯远程（只能靠自建服务器+中继连到的）设备永远不成立——远程设备根本不会被 mDNS 发现，这类事件驱动逻辑需要一条独立的周期定时器兜底，不能假设"事件总会来"。
- **回环/CI 测试环境没有真实 NAT，会让"打洞"这类优化性质的连接阶梯档位在测试里显得比生产环境更强势**（C5 教训）：C3 写 `forced_relay_path_completes_a_transfer` 时，"强制走中继"的手法（关 mDNS + 钉死地址逼前两档失败）在当时是对的——那会儿还没有打洞。等 C5 把打洞插到中继前面，同一套强制手法就不再能保证测到"中继"了，因为打洞在回环环境下会稳定成功、抢在中继之前把连接接上，而测试本身只断言了"传输成功"，从没检查过走的是哪一档，所以这个回归**在测试绿灯的情况下悄悄发生了**，直到这次专门去追查才发现。**教训**：给"连接阶梯第 N 档"这类有明确优先级、且后加的档位可能截胡先加的档位的测试，光断言最终结果（文件到了）是不够的，必须断言"走的是哪一档"（这里用 `ConnectionVia` 事件）；新增一档时，第一件事是检查有没有现存测试隐含假设了"这一档不存在"。
- **QUIC 的 `Connection` 句柄和从它派生出的 `RecvStream`/`SendStream` 生命周期不是自动绑定的**（C5 教训，但影响面回溯到 C1）：`quic::connect()`/入站 accept 拿到 `(send, recv)` 后，如果只把这两个流传下去、让本地 `connection: quinn::Connection` 变量自己随函数返回而丢弃，流仍然能用（因为 quinn 内部有自己的引用计数），但如果调用方紧接着又立刻返回（比如"转交给钩子后不等它跑完"这种 fire-and-forget 分流），`Connection` 句柄计数可能提前归零，连接被拆得比数据真正发送完还早。**排查方法**：先怀疑协议层握手逻辑（对照 `client_hello`/`server_hello` 双方日志确认握手本身没问题），确认握手成功但后续读写报 "connection lost" 后，才想到去查"谁在什么时候丢了 Connection 对象"——加 `eprintln!` 打点到每个可能提前返回的路径，能看到「写完 → 函数返回 → 紧接着才报错」的时序。**根治方案**：让承载流的类型自己拿着 `Connection` 一起走（本例是给 `QuicDuplex` 加一个 `_connection` 字段），不要指望每个调用点都记得"顺手多存一个变量"。**配套教训**：写完最后一条消息就直接返回也不安全——"写成功"只代表数据进了本地发送缓冲区，不代表已经送达对端，紧接着丢连接可能把还没发出的字节冲掉；需要显式半关闭写侧、读到对端也关闭为止，才能确认数据交接完毕。
- **打破一个"从未被打破过的隐性假设"时，要主动去找所有依赖它的地方，而不是等它报错**（C6 教训）：`transfer_tasks.peer_device_id REFERENCES devices(id)` 这个外键从 V0.1 建表起就在，此前"peer 必然是已配对设备"从未被打破过（`Offer`/`FetchRequest`/`IndexRequest` 都要求 `trusted`），所以从没人验证过"peer 未知时会怎样"。`ShareRequest` 允许未配对设备访问后，第一次踩进这个假设——但 bug 不是一次性暴露的：`serve_fetch`/`fetch::drive` 各自有**两处**依赖同一个前提的数据库写入（`insert_task` 和后续的 `update_task_status`），改第一处后测试换了个新错误（同类根因，不同代码位置），改完第一处以为修好了，跑测试才发现还有第二处——这是"打了地鼠才发现还有一只"的典型模式。**排查方法**：给整条链路（`relay_dial` → `spawn_relay_accept` → `accept_external` → `dispatch_shared` → `serve_fetch`/`fetch::drive`）临时加 `eprintln!` 逐段打点，而不是只看最外层的错误信息（"connection lost" 完全没提示真正原因是数据库外键，因为错误发生在远端，本地只看到连接异常关闭）。**教训**：遇到"这个假设是不是第一次被打破"的场景，与其头痛医头改一处报错再等下一处报错，不如先搜一遍这个字段/表在所有写入路径里的用法（`grep -n "insert_task\|update_task_status" crates/aa4c-transfer/src/*.rs`），一次性确认哪些调用点共享同一个前提。**根治方案的取舍**：没有改外键约束本身（会连带改变"解除配对级联删除历史记录"这个既有 V0.1 行为，风险面更大），而是在调用点判断"对端是否已知"来决定是否落库——牺牲的是"未配对访问不出现在传输记录页"，换来零行为改动。
- **第三方进程的"优雅关闭"可能自带你不知道的内部延迟**（D1 教训）：设计阶段假设"RPC 发一条 shutdown 指令 + 短暂宽限期 + 超时强杀"就够了，宽限期直觉上设了 2 秒。真机联调时下载任务反复无法验证"是否真的优雅落盘"，一开始怀疑是自己的 RPC 客户端有 bug，加了一堆 `eprintln!` 排查（甚至一度怀疑是 tokio 单线程 runtime 调度问题），最后发现是 aria2 自己的 `aria2.shutdown`（区别于 `forceShutdown`）内部就会等**约 3 秒**才真正退出（日志明确打印"3 second(s) has passed. Stopping application."）——这是 aria2 的既定行为，不是我们能改的，2 秒宽限期系统性地"抢跑"在它完成 session 落盘之前发生。**教训**：包一层外部工具时，"优雅关闭需要多久"不要凭直觉估一个数字，尤其涉及数据落盘的场景——如果文档没写清楚，直接跑一次真实的关闭流程、看它自己的日志输出，比调试自己的代码更快找到真相。
- **测试如果不显式隔离"进程会实际写文件的目录"，早晚会污染开发者的真实文件系统**（D1 教训，人工走查中发现）：`aa4c-core` 的下载端到端测试用了真实 `ProcessSpawner` + 真实 aria2c，但没有像 `transfer.default_save_dir` 那样显式覆盖下载目录——`Settings.download_dir` 没被覆盖时会读到 `default_download_dir()` 的默认值（系统真实 Downloads 目录），于是每跑一次这个测试就会真的在开发机 `~/Downloads` 里留下一个文件。这个 bug 在纯自动化测试运行中完全不会被发现（CI 的 `~/Downloads` 是一次性容器，没人会去看），只有在真人做真机联调、恰好瞟了一眼自己的下载文件夹时才会注意到。**教训**：任何"这段代码会执行真实副作用（写文件/发网络请求/改系统状态）"的测试，都要显式检查它用的路径/地址是不是打点在隔离的临时目录里——不能假设"用了 tempdir 作为 data_dir 就完全隔离了"，因为某个字段（这里是 `download_dir`）完全可能来自另一条独立的默认值链路，压根没被 tempdir 覆盖到。**修法**：`Core::start()` 之前，直接向同一个 `aa4c.db` 文件预置一条 `settings` KV（`Store::open` + `set_setting`），确保 `settings::load()` 在 Core 真正启动、`DownloadService` 生成 aria2 conf 文件之前就读到隔离路径——`update_settings()` 太晚了，`DownloadService::start()` 在 `Core::start()` 内部就已经把 `dir=` 写死进 conf 文件了。
- **一份从 WebFetch/文档摘要拼出来的 Tauri capability JSON，正确性只有真机跑起来才算数**（D1 教训）：`shell:allow-execute` + `sidecar:true` + 参数 `validator` 正则的精确字段名，没有任何一次 WebFetch 查证给出过完全一致、可信引用源码的答案（不同请求给出的字段名甚至互相矛盾），最后是凭对 Tauri v2 shell 插件 `scope.rs` 大致结构的推断写的。这类"格式高度特定、光靠读文档/摘要很难 100%确定"的配置，宁可老老实实跑一次真实场景（`pnpm tauri dev` + 观察 sidecar 是否真的拉起来），也不要满足于"读起来像是对的"就直接认定完成——真机走查这次直接给出了决定性证据（日志里出现"download engine (aria2c) connected"，说明 capability 权限确实放行了）。
- **`bundle.externalBin` 不区分桌面/移动端目标——声明后 Android 构建也会去校验那个文件存在**（D1 教训，切 `v0.4.0-preview` 时在真实 CI 里踩到，不是本机能提前发现的）：本机验证过 `cargo build -p aa4c-desktop`（桌面 target）没问题，但从没试过 `pnpm tauri android build`——直到 release CI 里 Android APK job 真的失败，报 `resource path binaries/aria2c-aarch64-linux-android doesn't exist`，才意识到 `tauri_build::build()` 对 `externalBin` 的资源校验是**按当前编译目标的 triple 查文件**，不管这个 sidecar 逻辑上是不是"只在桌面用"（Android 端 `desktop_download_spawner` 在代码里已经 `#[cfg(not(desktop))]` 返回 `None`，但这个 Rust 层的"不使用"挡不住 build.rs 的资源存在性检查，两者是完全独立的两套机制）。**教训**：给桌面/移动共用同一份 `tauri.conf.json` 的项目，新增任何桌面专属的 `bundle` 字段（尤其 `externalBin`）后，必须换算一下"这个字段是否也会被 Android/iOS 构建路径读到"——不能因为业务逻辑上做了平台判断就假设配置文件也自动跟着区分平台。**修法**：新建 `apps/desktop/src-tauri/tauri.android.conf.json`（`{"bundle":{"externalBin":[]}}`），Tauri 官方支持的平台专属配置覆盖文件命名规则（`tauri.<platform>.conf.json` 合并进主配置），针对 Android 构建把这个数组清空。同一个 CI 里 `ci.yml` 的 "Android (build sentinel)" job 一直有 `continue-on-error: true`，所以之前没有阻塞任何合并——只是哨兵一直在静默失败没人发现，这次顺手修了。
- **Windows 上 `choco install aria2` 装的不是真正的可执行文件，是一个靠"调用者工作目录"找真身的 shim——第一版诊断（怀疑 shell/PATH 传播时机）是错的，多花了两轮 CI 才找到真根因**（D1 教训，切 `v0.4.0-preview` 时在真实 CI 里踩到）：下载端到端测试在 Windows CI 上报 `Unavailable("download engine not available")`，第一反应是"`choco install` 之后 PATH 更新有传播延迟，`cargo test`（Windows 默认 `pwsh`）看不到刚装的东西，而紧邻的 `shell: bash` 步骤能看到"——这个假设**看起来很合理**（bash 步骤确实成功了），加了 `shell: bash` 到 `cargo test` 那一步就推上去了，结果**测试报的还是一模一样的错误**，说明假设从根上就错了。真正线索来自更笨但更可靠的办法：给 `EngineChild` 加一个 `recent_stdio()`，把子进程的 stdout/stderr 都截留最后 20 行，健康检查失败时打出来——第一次只截了 stderr，结果是空的（又是一次基于"合理猜测"的弯路：以为错误信息会走 stderr，实际 aria2 的 NOTICE/ERROR 日志走的是 stdout，跟 D1.9 真机走查时 `Stdio::inherit()` 看到的现象其实是同一个信号，当时没留意）；把 stdout 也截了之后，第一次真正看到了 aria2 自己吐出的错误：`Cannot find file at '..\lib\aria2\tools\aria2-1.37.0-win-64bit-build1\aria2c.exe'`——choco 装的 `aria2c.exe` 是 chocolatey 的 shim 机制生成的转发器，它内部用一个相对于**调用它的进程当前工作目录**的相对路径去找真正的二进制，而不是相对于 shim 自己所在的目录；从 `cargo test`（工作目录是仓库/crate 根）调用，这个相对路径解析到一个不存在的地方，shim 直接退出、什么都不监听，RPC 连接自然是 "connection refused"。**更严重的连带发现**：`release.yml` 打包 Windows 安装包时用的是同一套 `choco install` + `fetch-engines.sh --from-path`（复制 `command -v aria2c` 找到的文件）——`v0.4.0-preview` 已经发布的 Windows 安装包大概率打包了这同一个坏掉的 shim（构建本身不会报错，因为复制一个文件不需要执行它，只有真用户点下载才会在运行时炸）。**修法**：Windows 上完全不用 choco，改成直接下载官方 aria2 Windows release zip、解出真正的 `aria2c.exe`、把它所在目录加进 `$GITHUB_PATH`（`ci.yml`/`release.yml` 都改了）；顺带撤销了那个基于错误假设的 `shell: bash` workaround（不再需要）。**这次教训的教训**：一个"看起来合理、且不需要太多验证就能编出解释"的假设（PATH 传播时机），如果第一次尝试的修复没有让问题消失，不要急着找第二个同样"听起来合理"的假设去套——应该先让系统自己告诉你发生了什么（这里就是子进程的真实输出），而不是靠猜第二次、第三次。
- **`engines.yml` 首次真实运行，workflow 注释里"未经验证"的预判风险全部命中，外加两个写作时完全没想到的新坑——四轮 `workflow_dispatch` 才全绿**（D1 教训，`v0.4.0-preview.1` 发布后补跑）：①macOS x86_64 用 `clang -arch x86_64` 在 arm64 runner 上交叉编译，链接报一长串 `symbol(s) not found for architecture x86_64`——根因是 Homebrew 在 arm64 runner 上只装 arm64 单架构的 c-ares/libssh2 等依赖库，交叉编译产物根本链不上；改成两个架构各用原生 runner 编译。②修复①时随手选的 `macos-13` 标签**已经在 2025-12-04 被 GitHub 完全退役**——退役的标签不会报错，job 就是无限期 `queued` 排不到 runner，第一次遇到时排了 30+ 分钟毫无进展，一度以为是"资源紧张排队久"，后来专门查 GitHub 的 runner-images 仓库 issue 才确认是标签本身失效；换成当时仍受支持的 `macos-15-intel`。③Linux 静态构建 `autoreconf -i` 报 `Can't exec autopoint`，装了 `gettext` 包以为够了，结果**同样的报错又出现一次**——`apt-cache search` 才发现 Ubuntu 把 `autopoint` 拆成了独立于 `gettext` 的另一个包（版本号一样，`.deb` 不同），必须显式装。④解决③后进入链接阶段又报 `cannot find -llzma`/`cannot find -lcares`——前者是压根没装 `liblzma-dev`（apt 列表原来就漏了这一项）；后者更麻烦：Ubuntu 的 `libc-ares-dev` 只带 `.so` 不带静态 `.a`，`ARIA2_STATIC=yes` 要求所有依赖都有静态版可链，没有对应的"静态版"包可装，最后选择 `--without-libcares` 直接放弃这个依赖（异步 DNS 解析退化成标准同步解析，走系统 resolver，对下载中心 sidecar 场景没有实际影响，比自建静态 c-ares 划算得多）。⑤顺带在本机验证 `scripts/fetch-engines.sh` 真实下载模式时还发现脚本自己的一个 bug：正式下载分支的 `curl -o` 覆盖一个此前 `--from-path` 模式留下的只读文件（继承自 Homebrew 装的 aria2c 权限位）会报 "Failure writing output to destination"——`--from-path` 分支自己有 `rm -f` 前置清理，正式下载分支当初漏写了同一处防御，补上了。**教训**：一份"手写、从没在真实 CI 跑过"的 workflow，哪怕作者已经在注释里认真列出了自己能想到的风险点，实际跑起来大概率还会撞见作者没想到的新坑（这里的 runner 标签退役、Ubuntu 包拆分、静态库缺失都不在原始风险清单里）——**"跑过一次官方 README 推断出的步骤"和"这份 workflow 真的在这套 CI 环境里跑通过"是两件事**，前者只能降低意外的概率，不能替代后者。查一个第三方 CI 环境的"当前状态"（这里是 GitHub hosted runner 的标签是否还有效）比凭经验/凭旧知识猜更可靠——`actions/runner-images` 仓库的 issue 区就是这类信息的权威来源。
- **`tauri_plugin_shell::process::CommandChild` 只公开 `write`/`kill`/`pid` 三个方法，没有给 spawn 时注入钩子的余地**（D2 教训，孤儿进程防护设计阶段）：Linux 的 `PR_SET_PDEATHSIG` 必须在子进程 `exec` 前调用（`pre_exec` 钩子的固有约定），这对 `ProcessSpawner`（直接用 `tokio::process::Command`）毫无问题，但一开始想当然地以为 `TauriSidecarSpawner` 也能照搬——去查了 `CommandChild` 的实际公开 API（用 WebFetch 查 docs.rs 页面）才发现它就是一个"只给你 PID，其他全部封装掉"的句柄，没有任何等价于 `pre_exec` 的口子。**教训**：给一个第三方封装类型设计"需要在特定生命周期时机注入行为"的方案之前，先去确认这个类型的公开 API 到底暴露了什么，不要假设"直接用底层 API 能做的事，套壳之后也一定能做"——这类封装存在的意义往往就是收窄暴露面，越是看起来该有的钩子，越可能被设计者故意去掉了。**应对**：改成按能力分层——Windows 的 Job Object 方案本来就是"按 PID 事后归属"，不依赖 spawn 时钩子，天然对两条路径（`ProcessSpawner`/`TauriSidecarSpawner`）都适用；Linux/macOS 在 Tauri 路径下退化用 PID 文件方案（跟 macOS 完全一样的兜底），只有 `ProcessSpawner`（测试/Docker/headless 场景）才用得上真正的 `PR_SET_PDEATHSIG`。
- **Windows Job Object 的 PoC 从写代码到真正跑绿，三轮都是"表面上合理，编译器/运行时告诉你哪里错了才知道"的具体小问题，不是设计错了**（D2 教训，孤儿进程防护小样验证）：①第一次编译报 `CreateJobObjectW` 在作用域里找不到——以为是 `windows-sys` 用法写错了，实际是漏了 `Win32_Security` feature（`CreateJobObjectW` 的参数类型 `SECURITY_ATTRIBUTES` 定义在那个 feature 门后面，windows-sys 按类型门控代码生成，不是所有 `Win32_System_JobObjects` 下的函数都只需要那一个 feature）。②修完①又报类型不匹配——`job != 0` 编译不过，这个版本的 `windows-sys` 里 `HANDLE` 是 `*mut c_void` 不是 `usize`，改成 `!job.is_null()`。③两个编译问题都修完后 CI 第一次显示"失败"，但仔细看日志发现测试脚本自己打印了 "PASS: child pid=... died automatically"——机制本身其实早就工作了，只是 PowerShell 包装脚本没有清空 `poc.exe`（用 `abort()` 模拟崩溃退出）残留的 `$LASTEXITCODE`，这个残留值把整个 step 判成失败，纯粹是脚本包装层的 cosmetic bug。**教训**：多轮 CI 失败不代表核心机制有问题——每次失败先完整读日志找真正的错误信息（尤其留意"测试逻辑自己打印的结果"和"这次 CI job/step 的退出码"其实是两件独立的事，可能互相矛盾），不要看到"failure"红标就假设需要重新设计，很多时候只是一层薄薄的、离核心逻辑很远的包装出了问题。

V0.3「AA Connect」六个里程碑（C1 QUIC + 断点续传、C2 `aa4c-server` 信令面、C3 Relay 中继、C4 远程同步/发送 + 连接质量、C5 NAT 打洞、C6 分享链接）**全部实现并测试通过，且已打包发布 `v0.3.0-preview`**——连接阶梯「局域网直连 → 公网直连 → 打洞 → 中继」四档全部贯通，外加脱离配对关系的能力型分享，随三平台安装包/APK/服务器二进制发出。**V0.4「Download」设计已定稿（v2）、实现计划已产出**（[DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) + [V0.4_IMPLEMENTATION_PLAN.md](V0.4_IMPLEMENTATION_PLAN.md)），尚未开始实现。**下一步是既定任务：按计划实现里程碑 D1（Aria2 集成）**，按计划步骤 1–10 顺序走，决策依据在 DOWNLOAD_DESIGN §9，不要重开已定案的讨论。
