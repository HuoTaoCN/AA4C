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
- V0.3+：QUIC、NAT 穿透、Relay
- 分块传输 + BLAKE3 哈希校验 + 断点续传

### Sync Service（V0.2+）

负责持续同步、增量同步、版本管理、冲突处理。设计参考 Syncthing 的块交换模型。

### Download Service（V0.4，里程碑 D1 已实现，见 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md)）

负责 HTTP / HTTPS / FTP（D1，Aria2 RPC）与 BT / Magnet（D2，Transmission RPC——v3 从 qBittorrent 换过来，理由见 DOWNLOAD_DESIGN.md §3.6.1）下载；S3 后续评估。两个引擎都作为 AA4C 自动打包/管理的独立子进程运行（GPL 许可证隔离，只通过 RPC/API 调用，不链接源码），与 Core 之间用 `SidecarSpawner` trait 解耦，Core 本身不直接依赖 Tauri 专属的进程拉起 API。站点化长尾需求（私有 Tracker/PT、搜索、自动分类）预留 Lua 插件系统（DOWNLOAD_DESIGN.md §10，V0.4 之后的独立里程碑）。

### AI Service（V0.5+）

负责文件分类、自动标签、知识库管理、向量索引。基于 llama.cpp 运行本地 GGUF 模型，未来支持 Agent。

### Storage Service

负责元数据、任务状态、设备信息、同步记录的持久化。V0.1 使用 SQLite，详见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)。

## Plugin System

目标：所有高级能力插件化。

插件接口（Rust trait）：

```
Plugin（基础生命周期）
├── DownloadPlugin
├── SyncPlugin
├── StoragePlugin
├── AIPlugin
└── SharePlugin
```

第三方可基于 Plugin API 开发 NAS 插件、云盘插件、模型插件、知识库插件。

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
| `aa4c-types` | 公共类型 | DeviceInfo、TransferTask、错误类型、事件 |
| `aa4c-proto` | 线路协议 | Message 定义、帧编解码（配对与传输共用） |
| `aa4c-core` | Core | 生命周期、事件总线、配置、服务编排 |
| `aa4c-identity` | Security | 设备密钥、证书、配对协议 |
| `aa4c-discovery` | Device Service | mDNS 发现 |
| `aa4c-transfer` | Transfer Service | 传输协议与引擎 |
| `aa4c-store` | Storage Service | SQLite 持久化 |
| `aa4c-download` | Download Service | aria2 引擎子进程生命周期 + JSON-RPC 客户端（V0.4 里程碑 D1） |
| `apps/desktop` | Desktop + Android UI | Tauri 2 + Vue3（Android 工程由 Tauri 生成于 `src-tauri/gen/android`） |

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
