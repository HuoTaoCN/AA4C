# AA4C 开发交接（换机指南）

> 最后更新：2026-06-13。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
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
| **A1 Android 适配** | ⬜ 下一个 | — | 多播锁 Kotlin 插件、数据目录注入、Manifest 权限、文件选择 |
| M8 / A2–A3 | ⬜ | — | 联调发布 / Android 适配收尾 |

整个 V0.1 桌面端链路 **发现 → 配对 → 传输 → UI** 已全部打通；下一步进入 Android 平台适配（A1，前置 A0+M6 均已就绪）或桌面联调发布（M8）。

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

## 四、下一步：A1 Android 适配 或 M8 桌面联调发布

V0.1 桌面端功能已闭环（前端 + 后端）。两条路任选其一推进：

### 选项一：A1 Android 平台适配（前置 A0+M6 已就绪）

按 [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) A1 节与 [API_DESIGN.md](API_DESIGN.md) §11：

- **多播锁**：Kotlin 插件在 `onCreate` 获取 `WifiManager.MulticastLock`、`onDestroy` 释放（否则 Android 收不到 mDNS 组播）；Rust 侧无改动
- **数据目录**：`CoreConfig.data_dir` 已由 Tauri `app.path().app_data_dir()` 注入（桌面/Android 同源），Android 指向应用私有目录——这部分 M6 已做对，确认即可
- **Manifest 权限**：`INTERNET`、`ACCESS_NETWORK_STATE`、`CHANGE_WIFI_MULTICAST_STATE`、（Android 13+）`POST_NOTIFICATIONS`
- **文件选择**：前端 AA 页已用 `tauri-plugin-dialog`（拖拽在移动端不可用，按钮已就位）；底部导航 < 700px 已切换
- 验收：真机 APK 与桌面端互相发现、配对、传输

### 选项二：M8 桌面联调、打包、发布

双真机联调（macOS ↔ Windows）、修联调 bug、`pnpm tauri build` 出三平台包、写发布说明。

### M7 前端自测要点（联调时用）

- `pnpm tauri dev` 启动；两实例应在首页互相出现，可配对、传输
- 前端代码在 `apps/desktop/src/`：`lib/`（api/events/format/types）、`stores/`（4 个 Pinia）、`pages/`（4 页）、`components/`（卡片/弹窗/任务条/toast）
- 新增依赖：vue-router、pinia、@tauri-apps/plugin-dialog、plugin-notification；对应 Rust 插件已在 `src-tauri` 注册，能力在 `capabilities/default.json`

## 五、本次会话的教训（务必遵守）

- **一次只跑一个后台测试任务**：之前同时挂多个重叠的 `cargo test` 后台任务，被 harness 标记 killed 后留下僵尸测试二进制（占 TCP 端口 / Store 线程），导致后续测试互相抢占、越来越慢甚至卡死。跑测试前先 `ps -eo pid,etime,command | grep aa4c_` 确认无残留。
- **`cargo test --workspace` 会跨 crate 并行跑测试二进制**，单独 `cargo test -p X` 过不代表 workspace 过。提交前务必跑一次完整 `cargo test --workspace`。
- **lib 内联单测 ≠ 集成测试**：`cargo test -p X --test Y` 只跑集成测试，漏掉 `src/*.rs` 里的 `#[cfg(test)]`。要 `--lib` 或直接 `--workspace` 覆盖全部。

对 Agent 直接说"**开始 A1**"（Android 适配）或"**开始 M8**"（桌面联调发布）即可继续。
