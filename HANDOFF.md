# AA4C 开发交接（换机指南）

> 最后更新：2026-06-30。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
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
| **V0.3 设计评审修订 + 实现计划** | 🚀 进行中 | — | 设计定稿 v2：服务器身份=密钥对+地址内指纹、允许名单+挑战应答取代互签 proof、单进程 `aa4c-server`、信令复用帧层 bincode（弃 HTTP/WS）、单 `server_url` 默认关、分享仅已索引内容、**中继提前到打洞前**；新增 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)（C1–C6，C1 细化到可直接执行）。仅设计未实现 |

整个 V0.1 桌面端链路 **发现 → 配对 → 传输 → UI** 已全部打通。**V0.2 同步五个里程碑（信任分级 / 本地索引 + Inbox / 跨设备索引交换 + 统一视图 / 按需拉取 / 冲突标记）全部落地**（SYNC_DESIGN.md §10）；线路协议已升到 `proto=2` 并对同步路径按版本 gate（与 v0.2.0-preview 的同步不再互通，趁预发布窗口对齐）。**真机 GUI 走查已人工跑通**（`scripts/dev-two-nodes.sh` 起两实例：配对 → 互标我的设备 → 黄「可下载」→ 点黄拉取转绿 → 同名不同内容「多版本」并列，均正常）。

### 已实现 crate 概览（`crates/`）

| crate | 职责 | 关键公共 API |
|-------|------|--------------|
| `aa4c-types` | 公共类型 | `DeviceInfo` `TransferTask` `CoreEvent` `Aa4cError`（含 `code()`）；常量 `DEFAULT_PORT=42420` `CHUNK_SIZE` `MAX_FRAME_LEN` |
| `aa4c-proto` | 线路协议 | `Message` 枚举、`read_message`/`write_message`/`encode_frame`、`client_hello`/`server_hello` |
| `aa4c-identity` | 身份 + 配对 | `Identity::load_or_generate`、`tls_server_config`/`tls_client_config`（mTLS 证书固定）、`derive_pin`、`PairingManager`（`start_pairing`/`handle_incoming`/`confirm`） |
| `aa4c-discovery` | mDNS | `DiscoveryService::new/start/stop/devices` |
| `aa4c-store` | SQLite | `Store::open`、设备/任务/设置 CRUD（`Store` 是廉价克隆句柄，内部专职线程） |
| `aa4c-transfer` | 传输 + 索引交换 + 按需拉取 | `TransferService::new`（返回 `Arc<Self>`）、`start_listener`/`send`/`accept`/`cancel`/`fetch_index`/`fetch_file`；`set_pair_dispatch` / `set_index_dispatch` / `set_fetch_resolver` 注入钩子（`IncomingPairDispatch` / `IncomingIndexDispatch` / `SharedFileResolver` trait）；推送与拉取共用 `recv::receive_files` + `send::serve_fetch` |
| `aa4c-core` | 组装 | `Core::start`/`shutdown`/`subscribe`/`self_info`/`listen_port`；§9 的 11 个 Command 在 Core 上有同名编排方法；`CoreConfig`、`Settings` 读写 |

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

## 四、下一步：V0.3 里程碑 C1（QUIC 会话层 + 断点续传）

**V0.2 已全部完成并发布**（`v0.2.0-preview.2`，CI 全绿）。**V0.3 设计已定稿（v2，经评审修订）**：[CONNECT_DESIGN.md](CONNECT_DESIGN.md)（§12 是已确认决策清单，**不要重开已定案讨论**）。实现拆解见 **[V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)**（C1–C6；顺序已定：**中继 C3 先于打洞 C5**，远程可用在 C3 成立）。

- **下一步 = C1**：QUIC 会话层（quinn，证书固定复用，**单流等价迁移**——收发循环已泛型化可直接复用）+ `ResumeReport` 断点续传（**追加变体，绝不改既有 `Offer`**）。不依赖服务器，两端手填地址验证。计划文档里 C1 有步骤级拆解 + 验收清单 + 范围外清单，照做即可。
- C1 最大风险：**quinn 与现有 tokio-rustls 的 rustls 版本/加密后端对齐**，动手前先看 `Cargo.lock`。
- **可随时补的 V0.2 尾巴**（不阻塞 V0.3）：Inbox 按来源设备+时间分组、`IndexSummary` 摘要优化、冲突版本历史 / 自动合并。

> ⚠️ 版本兼容：`v0.2.0-preview.2`（proto=2）与 `v0.2.0-preview`（proto=1）**跨设备同步不互通**；与 v0.1.x 因 `DeviceInfo.trust_level` 无法配对。C1 起 proto 升 3，仍向后兼容降级。

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

V0.2 已全部实现并发布；V0.3 设计定稿 v2 + 实现计划就绪。对 Agent 直接说"**开始 V0.3 里程碑 C1**"即可继续——按 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md) 的 C1 步骤执行（QUIC 单流迁移 + ResumeReport 续传，先手填地址跑通，勿动 CONNECT_DESIGN §12 已定案项）。
