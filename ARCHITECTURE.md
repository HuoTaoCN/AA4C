# AA连接（AA4C）Architecture

## Overview

AA连接（AA4C）是一个**跨平台设备连接平台**——架构遵循"设备 → 连接 → 能力"：连接层是地基，文件传输 / 同步 / 分享 / 下载 / AI 归档都是连接之上的能力（Service / Plugin）。

采用 **Core + Service + Plugin + Transport + Storage** 分层架构，目标是支持未来十年以上的持续扩展。

核心设计原则：

1. **Core 只协调，不实现业务** —— 业务能力全部下沉到 Service 与 Plugin
2. **每个功能独立 Service** —— 低耦合、可独立测试、可独立替换
3. **高级能力插件化** —— Download / BT / AI / 云盘等通过统一 Plugin API 接入
4. **UI 与 Core 分离** —— Core 是纯 Rust 库，可被 Tauri（桌面 + 移动）、Docker（HTTP）复用；桌面端与 Android 共享同一 Tauri 工程与 Vue3 前端

## 整体架构

```
┌──────────────────────────────────────────────────────┐
│                       AA4C UI                        │
│  ┌────────────┐   ┌────────────┐   ┌────────────┐    │
│  │  Desktop   │   │   Mobile   │   │    Web     │    │
│  │ Tauri+Vue3 │   │ Tauri+Vue3 │   │  Vue3(可选) │    │
│  └─────┬──────┘   └─────┬──────┘   └─────┬──────┘    │
└────────┼────────────────┼────────────────┼───────────┘
         │ Tauri IPC      │ Tauri IPC      │ HTTP API
┌────────┴────────────────┴────────────────┴───────────┐
│                     AA4C Core (Rust)                 │
│        状态管理 · 生命周期 · 事件总线 · 配置          │
└────────┬────────────────┬────────────────┬───────────┘
         │                │                │
┌────────┴───────┐ ┌──────┴───────┐ ┌──────┴───────┐
│ Device Service │ │   Transfer   │ │     Sync     │
│  发现/配对/认证 │ │   Service    │ │   Service    │
└────────┬───────┘ └──────┬───────┘ └──────┬───────┘
         │                │                │
┌────────┴────────────────┴────────────────┴───────────┐
│                    Plugin Manager                    │
├──────────┬─────────┬─────────┬─────────┬─────────────┤
│ Download │   BT    │  Share  │   AI    │   Storage   │
└──────────┴─────────┴─────────┴─────────┴─────────────┘
┌──────────────────────────────────────────────────────┐
│   Transport Layer:  TCP / QUIC / NAT穿透 / Relay     │
├──────────────────────────────────────────────────────┤
│   Security Layer:   设备证书 / ECDH / AES-256-GCM    │
├──────────────────────────────────────────────────────┤
│   Storage Layer:    SQLite（元数据） + 文件系统       │
└──────────────────────────────────────────────────────┘
```

## Core

**职责**：统一管理设备、用户、任务、插件、配置。

**Core 不负责具体业务，只负责协调**：

- 应用生命周期（启动 → 加载配置 → 启动服务 → 关闭）
- 事件总线（服务之间、Core 与 UI 之间通过事件解耦）
- 服务注册与依赖注入
- 全局配置读写

## Services

### Device Service

负责设备发现、设备配对、设备认证、设备状态维护。

- 发现协议：mDNS / Zeroconf（局域网），UDP 广播作为兜底
- 配对：交换公钥 + PIN 码确认（详见 [API_DESIGN.md](API_DESIGN.md)）
- 状态：在线 / 离线 / 已配对 / 未配对

### Transfer Service

负责文件传输、文件夹传输、远程传输。

- V0.1：局域网 TCP（TLS 1.3 加密）
- V0.3（已实现）：QUIC 会话层、NAT 打洞、自建 Relay 兜底
- 分块传输 + BLAKE3 哈希校验 + 断点续传（proto ≥ 3）

### Sync Service（V0.2+）

负责持续同步、增量同步、版本管理、冲突处理。设计参考 Syncthing 的块交换模型。

### Download Service（V0.4，里程碑 D1–D3 已实现，见 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md)）

负责 HTTP / HTTPS / FTP（D1，Aria2 RPC）与 BT / Magnet（D2，Transmission RPC——v3 从 qBittorrent 换过来，理由见 DOWNLOAD_DESIGN.md §3.6.1）下载；S3 后续评估。两个引擎都作为 AA4C 自动打包/管理的独立子进程运行（GPL 许可证隔离，只通过 RPC/API 调用，不链接源码），与 Core 之间用 `SidecarSpawner` trait 解耦，Core 本身不直接依赖 Tauri 专属的进程拉起 API。站点化长尾需求（私有 Tracker/PT、搜索、自动分类）预留 Lua 插件系统（DOWNLOAD_DESIGN.md §10，V0.4 之后的独立里程碑）。

### AI Service（V0.5，里程碑 AI1–AI5 已实现，设计见 [ARCHIVE_DESIGN.md](ARCHIVE_DESIGN.md)）

负责文件分类、自动标签、知识库管理、向量索引。基于 llama.cpp（`llama-server` sidecar，MIT，官方预编译产物，OpenAI 兼容 HTTP）运行本地 GGUF 模型，完全本地、零云端调用；懒启动 + 空闲自停。规则式归档引擎住 `aa4c-archive`（F1 前住在 `aa4c-core::archive`），AI 引擎住独立 crate `aa4c-ai`，两者合成一个 `archive` 插件；sidecar 公共设施（spawner/孤儿防护）抽到 `aa4c-engine` 供 download/ai 共用。核心原则：**规则自动、AI 建议**——AI 输出永不直接驱动文件操作。未来支持 Agent。

### Storage Service

负责元数据、任务状态、设备信息、同步记录的持久化。V0.1 使用 SQLite，详见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)。

## Plugin System

**状态：已实现**（`aa4c-plugin` crate）。此前这一节写了三个版本、代码里一行都没有——
下载中心与归档/AI 是直接焊在 `Core` 上的，占了 `Core` 三分之一的编排代码、28 个设置项里
的 18 个、60 个 Tauri Command 里的 27 个。

### 边界划在哪

**连接层与能力层留在核心**：身份 / 发现 / 信任 / 连接阶梯 / 传输 / 同步 / 分享——
它们就是「设备连成一片」这件事本身。**其余一律是插件**。判据不是「重不重要」，
而是「拿掉它之后 AA4C 还是不是 AA4C」：拿掉下载，"我的几台设备连成一片"一点不少；
拿掉配对，整个产品就没了。

```
                    ┌─────────────────────────────────┐
                    │ 插件层  下载 · 归档与 AI · 知识库  │
                    ├─────────────────────────────────┤
                    │ 能力层  传输 · 同步 · 分享         │
                    ├─────────────────────────────────┤
                    │ 连接层  身份 · 发现 · 信任 · 阶梯  │
                    └─────────────────────────────────┘
```

### 契约

```rust
trait Plugin {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn start(&self, ctx: PluginContext) -> PluginFuture<()>;
    fn shutdown(&self) -> PluginFuture<()>;
    fn invoke(&self, method: String, payload: Value) -> PluginFuture<Value>;
    fn on_settings_changed(&self, settings: Value) -> PluginFuture<()>;  // 默认 no-op
    fn settings_schema(&self) -> Value;
}
```

`PluginContext` 交给插件的只有：共享的 `Store`、事件总线发送端、它自己的数据目录、
它自己的设置。**拿不到 `Core`、拿不到 `TransferService`、拿不到 `Identity`**——
依赖方向因此是单向的。

- **trait 住在独立的 `aa4c-plugin` crate**，不在 `aa4c-core` 里：插件要实现它就得依赖它，
  放核心里就成了 `core → download → core` 的循环。现在是 `core → plugin ← 各插件`。
- **编译期 trait，不做动态加载**：Rust 没有稳定 ABI，`dylib` 插件跨编译器版本就炸。
  第三方插件是 V1.0 的议题，载体应是进程外 RPC 或脚本（DOWNLOAD_DESIGN.md §10 预留的
  Lua 属于后者），不是 `dlopen`。
- **一个通用调用入口**（Tauri 的 `plugin_invoke(plugin, method, payload)`），不是每个插件
  一批类型化 Command：`generate_handler!` 是编译期展开的，按插件分组会让壳层长出一堆
  `#[cfg]`。代价是参数校验从编译期挪到运行期，由插件自己反序列化时负责。
- **数据库共用一条连接**（`Store::with_conn`），不给插件独立数据库：下载任务要能被归档
  规则读到、归档条目要能进同步索引，拆库就得自己造跨库事务。**迁移不随插件走**——
  `user_version` 是线性计数，摘出去要重编号，而重编号要重建表，那正是本项目栽过跟头的
  那类迁移（见 `aa4c-store/src/migrate.rs`）。

**壳层是唯一知道有哪些插件的地方**（`apps/desktop/src-tauri/src/lib.rs`）——只有它拿得到
基于 Tauri 的 sidecar 拉起器。空注册表 = 一个只有连接能力的构建，完全合法。

## Communication Layer

| 范围 | 协议 |
|------|------|
| 局域网发现 | mDNS、UDP 广播 |
| 局域网传输 | TCP（V0.1）、QUIC（V0.3+） |
| 广域网 | NAT Traversal（STUN / TURN）、Relay、BT |

## Security Layer

| 层 | 机制 |
|----|------|
| 设备身份 | Ed25519 设备密钥对，公钥指纹即设备 ID |
| 配对 | 公钥交换 + 短认证串（PIN）双向确认 |
| 通道加密 | TLS 1.3（自签名设备证书，证书固定到设备指纹） |
| 数据校验 | BLAKE3 文件哈希、分块哈希 |
| 权限 | 设备信任 → 目录授权 → 分享授权 |

## Code Layout（Rust Workspace）

| Crate | 对应模块 | 说明 |
|-------|----------|------|
| `aa4c-types` | 公共类型 | DeviceInfo、TransferTask、Settings、错误类型、事件 |
| `aa4c-proto` | 线路协议 | Message 定义、帧编解码（配对 / 传输 / 同步 / 服务器信令共用） |
| `aa4c-core` | Core | 生命周期、事件总线、配置、服务编排；`sync_*` 同步索引与交换、`server_link` 远程连接、`plugin` 注册表。**不依赖任何插件 crate** |
| `aa4c-identity` | Security | 设备密钥、TLS 证书与固定、配对协议、PIN 推导 |
| `aa4c-discovery` | Device Service | mDNS 发现与 TXT 解析 |
| `aa4c-transfer` | Transfer Service | 传输协议与引擎、QUIC 会话、断点续传、路径净化 |
| `aa4c-store` | Storage Service | SQLite 迁移与持久化 |
| `aa4c-server` | 信令 + 中继 | 自建 `aa4c-server`（库 + 二进制），设备注册 / 查询 / 中继（V0.3 里程碑 C2/C3） |
| `aa4c-plugin` | 插件边界 | `Plugin` trait + `PluginContext` + `PluginRegistry`。单独成 crate 是为了让依赖方向保持单向，见上文 Plugin System |
| `aa4c-archive` | 归档插件 | 文件类型识别、GGUF 元数据、规则引擎；连同 `aa4c-ai` 一起作为 `archive` 插件（F1 前住在 `aa4c-core::archive`） |
| `aa4c-engine` | Sidecar 公共设施 | 子进程 spawner 抽象与孤儿进程防护，供 download / ai 共用 |
| `aa4c-download` | 下载插件 | aria2（HTTP/FTP）与 Transmission（BT/磁力）子进程生命周期 + RPC 客户端（V0.4） |
| `aa4c-ai` | AI 引擎 | `llama-server` 子进程、OpenAI 兼容客户端、标签建议、知识库（V0.5）；经 `aa4c-archive` 接入插件边界 |
| `apps/desktop` | Desktop + Android UI | Tauri 2 + Vue3（Android 工程由 Tauri 生成于 `src-tauri/gen/android`） |

依赖方向严格单向：`types` → `proto` / `identity` / `store` → 各 Service → `core` → `apps`；
插件这一支是 `plugin` ← 各插件 crate ← `apps`，**`core` 永远不指向插件**。Core 只做编排、不写业务。

详细接口定义见 [API_DESIGN.md](API_DESIGN.md)。

## Supported Platforms

| 类型 | 平台 | 技术 |
|------|------|------|
| Desktop | Windows、macOS、Linux | Tauri 2 + Vue3 |
| Mobile | Android（V0.1 实验版）、iOS（后续） | Tauri 2（同一工程）；Flutter 为远期备选 |
| Server | Docker、NAS（ARM64 / x86_64） | Rust（无 UI） |

### Android 适配要点

- mDNS 收发需要持有 `WifiManager.MulticastLock`，通过 Tauri Android 插件（Kotlin）在应用启动时获取
- 接收目录默认使用应用专属外部存储（`Android/data/...`），导出到系统下载目录走 MediaStore
- 传输过程需前台服务（Foreground Service）防止系统杀进程（V0.2 完善）
- UI 响应式适配见 UI_DESIGN_SPEC.md §10
