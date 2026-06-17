# AA连接（AA4C）

**连接你的所有设备。 —— Connect all your devices.**

AA连接（AA4C = **AA for Connection**）是一个开源、跨平台的**设备连接平台**。先把你的所有设备连起来，文件传输、同步、分享、下载、AI 归档等能力都在连接之上自然发生——核心永远是**设备与设备的连接**。

> 把这个文件 AA 给我。
> 把照片 AA 到电脑。
> 把模型 AA 到服务器。

AA连接**不是**下载器、BT 工具、云盘、网盘、同步工具，也不是社区平台。它坚持：**用户数据属于用户自己**。

---

## 为什么叫 AA？

AA 是一个动作，代表：

| 动作 | 含义 |
|------|------|
| Send | 发送 |
| Sync | 同步 |
| Share | 分享 |
| Archive | 归档 |

AA 是连接设备后的核心动作，希望成为设备世界里的一个新动词。

## 核心理念

传统方式（数据经过第三方）：

```
手机 → 云盘 → 电脑
```

AA4C（设备直连，数据属于用户）：

```
手机 ↔ 电脑 ↔ NAS ↔ 服务器
```

- **点对点**：设备之间直接传输，不经过第三方服务器
- **本地优先**：数据保存在自己的设备上
- **默认加密**：所有通信端到端加密
- **用户控制**：不强制上传云端

## Features

| 能力 | 说明 | 状态 |
|------|------|------|
| **AA Send** | 局域网设备发现 + 点对点加密传输，支持文件/文件夹/大文件 | 🚧 V0.1 开发中 |
| **AA Sync** | 多设备文件夹同步，增量同步、冲突处理、版本管理 | 📋 V0.2 计划中 |
| **AA Share** | 指定好友 / 家庭 / 团队设备之间的分享（非社区） | 📋 V0.3 计划中 |
| **AA Download** | 统一下载中心：HTTP / HTTPS / FTP / BT / Magnet | 📋 V0.4 计划中 |
| **AA AI** | AI 分类、自动标签、智能搜索、本地知识库 | 📋 V0.5 计划中 |

## Platforms

| 平台 | 技术 | 系统 |
|------|------|------|
| Desktop | Tauri 2 + Vue3 | Windows / macOS / Linux |
| Mobile | Tauri 2（与桌面端同一代码库） | Android（开发中）/ iOS（计划） |
| Server | Docker | NAS / ARM64 / x86_64 |

## 安装与运行

### 安装包（推荐）

从 [Releases](https://github.com/HuoTaoCN/AA4C/releases) 下载对应平台的安装包：

| 平台 | 文件 | 说明 |
|------|------|------|
| macOS | `.dmg` | 通用包（Apple Silicon + Intel）。未签名，首次打开右键「打开」或在「系统设置 → 隐私与安全性」中放行 |
| Windows | `.msi` | 双击安装 |
| Linux | `.AppImage` | `chmod +x` 后直接运行 |
| Android（实验版） | `.apk` | 需在系统设置中允许「安装未知来源应用」 |

### 首次使用

1. 两台设备连接**同一个 WiFi / 局域网**
2. 都打开 AA4C，在首页即可看到彼此
3. 点对方卡片「配对」，两台屏幕核对 6 位确认码一致后确认
4. 在「AA」页选文件、选设备、点 AA

### 防火墙放行（找不到设备时先查这里）

AA4C 在局域网用 **TCP 42420**（传输）与 **UDP 5353**（mDNS 设备发现）通信：

- **Windows**：首次运行会弹防火墙询问，勾选 **「专用网络」** 放行；若误点取消，到「Windows Defender 防火墙 → 允许应用」里为 AA4C 勾选专用网络
- **macOS**：首次运行弹「是否允许接受传入连接」，点 **允许**
- **Linux（ufw 为例）**：`sudo ufw allow 42420/tcp && sudo ufw allow 5353/udp`
- 部分路由器 / 公司网络会隔离客户端或屏蔽组播，导致互相发现不到——换一个普通家用 WiFi 重试

### 从源码运行（开发）

环境与踩坑见 [HANDOFF.md](HANDOFF.md)。

```bash
cargo test --workspace                 # Rust 全绿
cd apps/desktop && pnpm install
pnpm tauri dev                         # 启动桌面端
pnpm tauri build                       # 打本平台安装包
```

## Architecture

```
UI（Tauri 2：桌面 + Android 同一代码库 / Web 可选）
        │
   AA4C Core (Rust)
        │
     Services
        │
     Plugins
```

详见 [ARCHITECTURE.md](ARCHITECTURE.md)。

## Documentation

| 文档 | 内容 |
|------|------|
| [PROJECT_VISION.md](PROJECT_VISION.md) | 产品需求与技术架构白皮书 |
| [ARCHITECTURE.md](ARCHITECTURE.md) | 总体架构设计 |
| [API_DESIGN.md](API_DESIGN.md) | Rust 模块接口设计 |
| [PROTOCOL.md](PROTOCOL.md) | AA 传输协议规范（v1 局域网 + v2 广域网草案） |
| [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) | SQLite 数据库设计 |
| [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) | UI 与 AA 交互设计规范 |
| [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) | V0.1 分步实现计划 |
| [TESTING.md](TESTING.md) | 测试策略与验收清单 |
| [ROADMAP.md](ROADMAP.md) | 开发路线图 |
| [AGENTS.md](AGENTS.md) | AI Agent 开发规则 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 贡献指南 |
| [HANDOFF.md](HANDOFF.md) | 开发交接：环境安装、踩坑注意、当前进度与下一步 |
| [SECURITY.md](SECURITY.md) | 安全策略与威胁模型 |
| [CHANGELOG.md](CHANGELOG.md) | 变更日志 |

## Status

**Project Stage: Early Development**

当前目标：完成 V0.1 —— 第一次 AA（设备发现、设备配对、局域网文件发送），桌面三平台 + Android 实验版并行开发。

## License

[Apache License 2.0](LICENSE)

## Community

- GitHub: https://github.com/HuoTaoCN/AA4C
- Website: https://aa4c.com
