# AA4C 开发交接（换机指南）

> 最后更新：2026-07-07（V0.3 C4 完成）。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
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

整个 V0.1 桌面端链路 **发现 → 配对 → 传输 → UI** 已全部打通。**V0.2 同步五个里程碑（信任分级 / 本地索引 + Inbox / 跨设备索引交换 + 统一视图 / 按需拉取 / 冲突标记）全部落地**（SYNC_DESIGN.md §10）；线路协议已升到 `proto=2` 并对同步路径按版本 gate（与 v0.2.0-preview 的同步不再互通，趁预发布窗口对齐）。**真机 GUI 走查已人工跑通**（`scripts/dev-two-nodes.sh` 起两实例：配对 → 互标我的设备 → 黄「可下载」→ 点黄拉取转绿 → 同名不同内容「多版本」并列，均正常）。

### 已实现 crate 概览（`crates/`）

| crate | 职责 | 关键公共 API |
|-------|------|--------------|
| `aa4c-types` | 公共类型 | `DeviceInfo` `TransferTask` `CoreEvent` `Aa4cError`（含 `code()`）；常量 `DEFAULT_PORT=42420` `CHUNK_SIZE` `MAX_FRAME_LEN` |
| `aa4c-proto` | 线路协议 | `Message` 枚举、`read_message`/`write_message`/`encode_frame`、`client_hello`/`server_hello` |
| `aa4c-identity` | 身份 + 配对 | `Identity::load_or_generate`、`tls_server_config`/`tls_client_config`（mTLS 证书固定）、`derive_pin`、`PairingManager`（`start_pairing`/`handle_incoming`/`confirm`） |
| `aa4c-discovery` | mDNS | `DiscoveryService::new/start/stop/devices` |
| `aa4c-store` | SQLite | `Store::open`、设备/任务/设置 CRUD（`Store` 是廉价克隆句柄，内部专职线程） |
| `aa4c-transfer` | 传输 + 索引交换 + 按需拉取 + QUIC + 中继 | `TransferService::new`（返回 `Arc<Self>`）、`start_listener`/`send`/`accept`/`cancel`/`fetch_index`/`fetch_file`/`accept_external`；`set_pair_dispatch` / `set_index_dispatch` / `set_fetch_resolver` / `set_relay_dialer` 注入钩子（`IncomingPairDispatch` / `IncomingIndexDispatch` / `SharedFileResolver` / `RelayDialer` trait）；推送与拉取共用 `recv::receive_files` + `send::serve_fetch`；`quic.rs` 会话层；`dial()`（`pub(crate)`）直连失败/无地址落中继兜底 |
| `aa4c-core` | 组装 | `Core::start`/`shutdown`/`subscribe`/`self_info`/`listen_port`；§9 的 11 个 Command 在 Core 上有同名编排方法；`CoreConfig`、`Settings` 读写；`server_link.rs`（自建服务器客户端接入：一次性 `register_once`/`lookup_once` + 常驻连接 `spawn_register_loop`，返回 `Notify` 供 `nudge_register` 立即唤醒重新注册） |
| `aa4c-server` | 自建信令 + 中继服务器（bin+lib） | `run(ServerConfig{data_dir, listen_addr}) -> Arc<Server>`；`Server::device_id`/`local_addr`/`address_with_host`；内嵌 `run()` 供测试驱动，供部署用 `main.rs`（`AA4C_SERVER_DATA_DIR`/`AA4C_SERVER_LISTEN` 环境变量）；中继面（`RelayRequest`/`RelayOpen` 等）随常驻连接的 `Register` 一并处理，无独立公开 API |

CI 现状：7 个 job 全绿（lint、三平台 test、frontend、audit、android 哨兵）。

## 二、新电脑环境安装（macOS）

### 必装（桌面轨开发，约 10 分钟）

```bash
# 1. Homebrew（如未装）https://brew.sh
# 2. Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# 3. Node 工具链
brew install node pnpm gh
# 4. gh 登录
gh auth login
# 5. 克隆并验证
git clone https://github.com/HuoTaoCN/AA4C.git && cd AA4C
cargo test --workspace            # Rust 全绿
cd apps/desktop && pnpm install && pnpm tauri dev   # 应出现 AA4C 欢迎窗口
```

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

## 四、下一步：V0.3 里程碑 C5（NAT 打洞，提速优化）

**V0.2 已全部完成并发布**（`v0.2.0-preview.2`，CI 全绿）。**V0.3 设计已定稿（v2）**：[CONNECT_DESIGN.md](CONNECT_DESIGN.md)（§12 是已确认决策清单，**不要重开已定案讨论**）。**里程碑 C1（QUIC 会话层 + 断点续传）、C2（`aa4c-server` 信令面）、C3（Relay 中继）、C4（远程同步/发送 + 连接质量）均已实现**：`cargo test --workspace` 全绿，fmt/clippy 干净，无回归。实现拆解见 **[V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)**（C1–C6；顺序已定：**中继 C3 先于打洞 C5**，远程可用已在 C3 成立，C4 起远程同步/发送/连接质量全线可用）。

- **下一步 = C5**：STUN 反射地址探测 + 服务器转发信令交换候选 + 双向同时发包打洞 → 成功后升级 QUIC 直连；这是**提速优化**，不是可达前提——失败直接落回已经在跑的中继（C3），不损可用性。需要新的 `ServerMessage::Signal`（打洞信令转发，只追加变体）。
- **C4 遗留、不阻塞 C5 的已知缩小范围**：`devices.server_hint` 已建表但配对协议未交换它，`resolve_peer`/`sync_exchange`/中继的 `RelayDialer` 目前都只查/连**自己配置的服务器**——跨服务器好友寻址还不可用，只覆盖「自己的多台设备」主场景；交换 server_hint 需要一条新的追加协议消息（`PairRequest`/`PairAccept`/`DeviceInfo` 是既有结构体，不能直接加字段），可以顺手做，也可以单独一个小里程碑。
- C1 遗留的两个小尾巴（不阻塞，随时可补）：keep-alive 目前用固定 8s 空闲超时+2s 心跳（已验证够用）；按需拉取（fetch）路径暂不支持续传（仅 Offer/send 路径支持）。
- **可随时补的 V0.2 尾巴**（不阻塞 V0.3）：Inbox 按来源设备+时间分组、`IndexSummary` 摘要优化、冲突版本历史 / 自动合并。

> ⚠️ 版本兼容：proto 现为 3（C1 起）；与 `proto=2`/`proto=1` 对端握手自动协商降级，行为不变（不发送 ResumeReport/QUIC 特有消息）。`v0.2.0-preview.2` 起的构建可与本版本互通同步；与 v0.1.x 仍因 `DeviceInfo.trust_level` 无法配对。
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

V0.2 已全部实现并发布；V0.3 里程碑 C1（QUIC + 断点续传）、C2（`aa4c-server` 信令面）、C3（Relay 中继）、C4（远程同步/发送 + 连接质量）均已实现并测试通过。对 Agent 直接说"**开始 V0.3 里程碑 C5**"即可继续——按 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md) 的 C5 小节执行（STUN 打洞 + 双向发包升级 QUIC 直连，纯提速优化，失败落回已有的中继；勿动 CONNECT_DESIGN §12 已定案项）。
