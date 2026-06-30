# AA4C 开发交接（换机指南）

> 最后更新：2026-06-29。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
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
| **V0.2 同步里程碑 2** | 🚀 进行中 | — | 共享范围 + 本地索引扫描 + Inbox 落点（`003_sync.sql`，`aa4c-core/src/sync_index.rs`），「同步」页接真实文件；尚未联调真机/未跑 `pnpm tauri dev` 实测 UI |

整个 V0.1 桌面端链路 **发现 → 配对 → 传输 → UI** 已全部打通。V0.2 进行中：信任分级数据模型已落地，本地同步索引（里程碑 2）刚完成，跨设备索引交换/同步（SYNC_DESIGN.md §10 里程碑 3–5）待续。

### 已实现 crate 概览（`crates/`）

| crate | 职责 | 关键公共 API |
|-------|------|--------------|
| `aa4c-types` | 公共类型 | `DeviceInfo` `TransferTask` `CoreEvent` `Aa4cError`（含 `code()`）；常量 `DEFAULT_PORT=42420` `CHUNK_SIZE` `MAX_FRAME_LEN` |
| `aa4c-proto` | 线路协议 | `Message` 枚举、`read_message`/`write_message`/`encode_frame`、`client_hello`/`server_hello` |
| `aa4c-identity` | 身份 + 配对 | `Identity::load_or_generate`、`tls_server_config`/`tls_client_config`（mTLS 证书固定）、`derive_pin`、`PairingManager`（`start_pairing`/`handle_incoming`/`confirm`） |
| `aa4c-discovery` | mDNS | `DiscoveryService::new/start/stop/devices` |
| `aa4c-store` | SQLite | `Store::open`、设备/任务/设置 CRUD（`Store` 是廉价克隆句柄，内部专职线程） |
| `aa4c-transfer` | 传输 | `TransferService::new`（返回 `Arc<Self>`）、`start_listener`/`send`/`accept`/`cancel`；`set_pair_dispatch` 注入配对分流钩子（`IncomingPairDispatch` trait）|
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
9. CI 的 android job 是 `continue-on-error` 哨兵，不阻塞合并；其余 job 必须全绿
10. `gh run watch` 经代理不稳定（annotations 接口 EOF），盯 CI 用轮询：
    ```bash
    gh api repos/HuoTaoCN/AA4C/actions/runs/<id>/jobs --jq '.jobs[] | "\(.name): \(.conclusion // .status)"'
    ```

## 四、下一步：V0.2 同步 —— 索引摘要交换 + 跨设备统一视图

里程碑 1（信任分级）、里程碑 2（共享范围 + 本地索引 + Inbox）已完成，「同步」页已接真实本机文件。下一步按 [SYNC_DESIGN.md](SYNC_DESIGN.md) §10 继续：

- **里程碑 3（下一步）**：索引摘要交换协议（`IndexSummary`/`IndexEntries`）+ `remote_index` + 统一视图（绿/黄/红，只读）——这一步上线后「可下载」(黄)/"设备离线"(红) 才会真正出现
- **里程碑 4**：按需拉取（复用现有 ATP `Offer`/分块传输）
- **里程碑 5**：冲突标记（同名不同 hash 加序号）与人工解决
- **未完成的里程碑 2 尾巴**（可随时补，不阻塞里程碑 3）：`notify` 文件系统实时监听（当前只有定时扫描 300s + 传输完成触发）、Inbox 按来源设备+时间分组展示
- **里程碑 2 尚未做的验证**：还没有跑 `pnpm tauri dev` 实际点过「添加同步文件夹」/「移除」/「重新扫描」按钮，只验证到 `cargo test --workspace` + `pnpm build`（类型检查）这一层——真机/真实 GUI 验证留给下一次会话或用户自测

> ⚠️ v0.2.0-preview 与 v0.1.x 配对协议不兼容（`DeviceInfo` 新增 `trust_level` 字段）。真机联调请确保两端都是 v0.2.0-preview 起的版本。

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

- `pnpm tauri dev` 启动；两实例应在首页互相出现，可配对、传输
- 前端代码在 `apps/desktop/src/`：`lib/`（api/events/format/types）、`stores/`（4 个 Pinia）、`pages/`（4 页）、`components/`（卡片/弹窗/任务条/toast）

## 五、本次会话的教训（务必遵守）

- **一次只跑一个后台测试任务**：之前同时挂多个重叠的 `cargo test` 后台任务，被 harness 标记 killed 后留下僵尸测试二进制（占 TCP 端口 / Store 线程），导致后续测试互相抢占、越来越慢甚至卡死。跑测试前先 `ps -eo pid,etime,command | grep aa4c_` 确认无残留。
- **`cargo test --workspace` 会跨 crate 并行跑测试二进制**，单独 `cargo test -p X` 过不代表 workspace 过。提交前务必跑一次完整 `cargo test --workspace`。
- **lib 内联单测 ≠ 集成测试**：`cargo test -p X --test Y` 只跑集成测试，漏掉 `src/*.rs` 里的 `#[cfg(test)]`。要 `--lib` 或直接 `--workspace` 覆盖全部。

对 Agent 直接说"**开始 V0.2 里程碑 3**"（索引摘要交换 + `remote_index` + 统一视图）即可继续，详见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §10。
