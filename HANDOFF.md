# AA4C 开发交接（换机指南）

> 最后更新：2026-06-12。用途：在新电脑上 `git clone` 后按本文档配置环境，即可无缝继续开发。
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
| **M6 Core + Tauri 桥** | ⬜ 下一个 | — | aa4c-core 组装（生命周期/事件总线/监听口分流配对与传输）+ 11 个 Tauri Command + 事件转发 |
| M7 前端 UI | ⬜ | — | 4 页面 + 弹窗 |
| M8 / A1–A3 | ⬜ | — | 联调发布 / Android 适配 |

最近 6 次 CI 全绿（三平台 fmt/clippy/test + 前端 + audit + Android 哨兵）。整个核心协议链路 **发现 → 配对 → 传输** 已打通并有测试覆盖。

### 已实现 crate 概览（`crates/`）

| crate | 职责 | 关键公共 API |
|-------|------|--------------|
| `aa4c-types` | 公共类型 | `DeviceInfo` `TransferTask` `CoreEvent` `Aa4cError`（含 `code()`）；常量 `DEFAULT_PORT=42420` `CHUNK_SIZE` `MAX_FRAME_LEN` |
| `aa4c-proto` | 线路协议 | `Message` 枚举、`read_message`/`write_message`/`encode_frame`、`client_hello`/`server_hello` |
| `aa4c-identity` | 身份 + 配对 | `Identity::load_or_generate`、`tls_server_config`/`tls_client_config`（mTLS 证书固定）、`derive_pin`、`PairingManager`（`start_pairing`/`handle_incoming`/`confirm`） |
| `aa4c-discovery` | mDNS | `DiscoveryService::new/start/stop/devices` |
| `aa4c-store` | SQLite | `Store::open`、设备/任务/设置 CRUD（`Store` 是廉价克隆句柄，内部专职线程） |
| `aa4c-transfer` | 传输 | `TransferService::new`（返回 `Arc<Self>`）、`start_listener`/`send`/`accept`/`cancel`；`recv.rs::run_incoming` 已留 PairRequest 分流占位待 M6 接线 |
| `aa4c-core` | 组装 | **空壳，M6 实现**：`Core::start`/`shutdown`/`subscribe` |

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

## 四、下一步：M6 Core 组装 + Tauri 桥

按 [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) M6 节执行，要点：

- `aa4c-core`：`Core::start` 启动序列（identity → store → transfer listener → discovery），`shutdown` 逆序优雅关闭，broadcast 事件总线接通所有服务
- **监听口分流**（M5 留的接口）：入站 TLS 连接握手后按首消息分流——`PairRequest` → `PairingManager::handle_incoming`，`Offer` → 传输会话。目前 `aa4c-transfer/src/recv.rs` 的 `run_incoming` 里 PairRequest 分支是占位（返回 reject），M6 要把它接到 PairingManager
- 启动时清理遗留任务（`waiting_accept/transferring` → `failed`）
- `src-tauri`：实现 [API_DESIGN.md](API_DESIGN.md) §9 全部 11 个 Command；`Core` 放入 Tauri `State`；`Core::subscribe()` → `app_handle.emit("aa4c://…")`，JSON camelCase
- Command 错误统一映射为 `{ code, message }`（`Aa4cError::code()` 已备好）

之后顺序：M7 UI → A1/A2 Android 适配 → M8/A3 发布。

## 五、本次会话的教训（务必遵守）

- **一次只跑一个后台测试任务**：之前同时挂多个重叠的 `cargo test` 后台任务，被 harness 标记 killed 后留下僵尸测试二进制（占 TCP 端口 / Store 线程），导致后续测试互相抢占、越来越慢甚至卡死。跑测试前先 `ps -eo pid,etime,command | grep aa4c_` 确认无残留。
- **`cargo test --workspace` 会跨 crate 并行跑测试二进制**，单独 `cargo test -p X` 过不代表 workspace 过。提交前务必跑一次完整 `cargo test --workspace`。
- **lib 内联单测 ≠ 集成测试**：`cargo test -p X --test Y` 只跑集成测试，漏掉 `src/*.rs` 里的 `#[cfg(test)]`。要 `--lib` 或直接 `--workspace` 覆盖全部。

对 Agent 直接说"**开始 M6**"即可继续。
