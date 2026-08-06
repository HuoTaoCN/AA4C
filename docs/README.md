# AA连接（AA4C）文档中心

> [English](en/README.md) · [返回项目首页](../README.md)

## 给使用者

| 文档 | 什么时候看 |
|------|------------|
| 📖 [用户手册](USER_GUIDE.md) | 想知道某个功能怎么用——传输、同步、分享、下载、归档与 AI、设置 |
| ❓ [常见问题与故障排查](FAQ.md) | 出问题了——找不到设备、传输失败、下载卡住、AI 用不了 |
| 🖥️ [自建服务器指南](SELF_HOSTING.md) | 想让不在同一网络的设备互相连接 |
| 🔓 [开源 · 开放 · 安全](OPEN_AND_SECURE.md) | 想知道数据去了哪里、隐私与安全怎么保证、许可证边界在哪 |

**新用户建议路径**：[README](../README.md) 快速开始 → [用户手册第 2 节](USER_GUIDE.md#2-第一次使用配对两台设备) 完成第一次配对 → 遇到问题查 [FAQ](FAQ.md)。

## 给开发者

开发与设计文档在仓库根目录（中文）：

| 文档 | 内容 |
|------|------|
| [PROJECT_VISION.md](../PROJECT_VISION.md) | 产品需求与技术架构白皮书 |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | 总体架构与 crate 划分 |
| [API_DESIGN.md](../API_DESIGN.md) | Rust 模块接口设计 |
| [PROTOCOL.md](../PROTOCOL.md) | AA 传输协议规范 |
| [DATABASE_SCHEMA.md](../DATABASE_SCHEMA.md) | SQLite 数据库设计 |
| [UI_DESIGN_SPEC.md](../UI_DESIGN_SPEC.md) | UI 与交互设计规范 |
| [TESTING.md](../TESTING.md) | 测试策略与验收清单 |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | 贡献指南 |
| [SECURITY.md](../SECURITY.md) | 安全策略与威胁模型 |
| [ROADMAP.md](../ROADMAP.md) | 开发路线图 |
| [HANDOFF.md](../HANDOFF.md) | 开发交接：环境、踩坑、当前进度 |
| [CHANGELOG.md](../CHANGELOG.md) | 变更日志 |

### 模块详细设计

| 文档 | 对应能力 |
|------|----------|
| [SYNC_DESIGN.md](../SYNC_DESIGN.md) | AA Sync：信任分级、跨设备索引、按需获取、冲突处理 |
| [CONNECT_DESIGN.md](../CONNECT_DESIGN.md) | AA Connect / Share：信令、中继、NAT 打洞、分享链接 |
| [DOWNLOAD_DESIGN.md](../DOWNLOAD_DESIGN.md) | AA Download：引擎集成、任务模型、限速与合规 |
| [ARCHIVE_DESIGN.md](../ARCHIVE_DESIGN.md) | AA Archive & AI：规则引擎、GGUF 解析、AI 建议、知识库 |
| [TOUCH_DESIGN.md](../TOUCH_DESIGN.md) | AA Touch / Direct：NFC、WiFi Direct、蓝牙（设计稿） |

### 实现计划（按版本）

[V0.1](../V0.1_IMPLEMENTATION_PLAN.md) · [V0.3](../V0.3_IMPLEMENTATION_PLAN.md) · [V0.4](../V0.4_IMPLEMENTATION_PLAN.md) · [V0.5](../V0.5_IMPLEMENTATION_PLAN.md) · [V0.6](../V0.6_IMPLEMENTATION_PLAN.md)

## 语言说明

用户文档提供中英双语（[English](en/)）。架构与协议等开发文档目前仅中文——欢迎贡献翻译，见 [CONTRIBUTING.md](../CONTRIBUTING.md)。
