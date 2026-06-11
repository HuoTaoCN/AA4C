# AA4C

**Let all your devices become one space. —— 让所有设备成为一个空间。**

AA4C 是一个开源、跨平台的个人数字空间系统。它将文件传输、文件同步、文件共享、下载管理、AI 归档统一到一个简单的动作 —— **AA**。

> 把这个文件 AA 给我。
> 把照片 AA 到电脑。
> 把模型 AA 到服务器。

---

## 为什么叫 AA？

AA 是一个动作，代表：

| 动作 | 含义 |
|------|------|
| Send | 发送 |
| Sync | 同步 |
| Share | 分享 |
| Archive | 归档 |

AA4C 希望成为文件世界里的一个新动词。

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
| **AA Share** | 临时/长期分享，好友设备、社区共享 | 📋 V0.3 计划中 |
| **AA Download** | 统一下载中心：HTTP / HTTPS / FTP / BT / Magnet | 📋 V0.4 计划中 |
| **AA AI** | AI 分类、自动标签、智能搜索、本地知识库 | 📋 V0.5 计划中 |

## Platforms

| 平台 | 技术 | 系统 |
|------|------|------|
| Desktop | Tauri 2 + Vue3 | Windows / macOS / Linux |
| Mobile | Tauri 2（与桌面端同一代码库） | Android（开发中）/ iOS（计划） |
| Server | Docker | NAS / ARM64 / x86_64 |

## Architecture

```
UI (Tauri / Flutter / Web)
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
