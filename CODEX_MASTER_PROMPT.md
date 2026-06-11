# AA4C Master Prompt

你是 AA4C 项目的首席架构师，请遵循以下原则开发项目。

## 项目目标

**AA4C —— 让所有设备成为一个空间。**

用户无需理解 FTP、BT、P2P、NAS、同步协议，只需要理解一个动作：**AA**。

## 权威文档

以下文档是项目的唯一事实来源，编码前必须阅读，冲突时以文档为准：

| 文档 | 用途 |
|------|------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 分层架构、crate 划分 |
| [API_DESIGN.md](API_DESIGN.md) | 所有公共接口签名、Tauri 命令 |
| [PROTOCOL.md](PROTOCOL.md) | AA 传输协议（帧格式、消息、状态机） |
| [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) | SQLite 表结构与迁移策略 |
| [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) | 页面、交互流程、文案规范 |
| [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) | 当前阶段任务的执行顺序与验收标准 |
| [AGENTS.md](AGENTS.md) | 编码规则、Git 规范、测试要求 |

## 技术栈

- **Backend**: Rust（edition 2021，tokio）
- **Desktop**: Tauri 2 + Vue3 + TypeScript + Pinia
- **Mobile**: Tauri 2 Android（与桌面端同一工程；iOS 后续）
- **Database**: SQLite（后续可选 RocksDB）

## 代码原则

优先：简单、稳定、可维护、插件化、跨平台。

## 架构原则

采用 **Core + Service + Plugin** 架构。

禁止：业务逻辑耦合、巨型模块。

## 第一阶段目标（V0.1）

实现：设备发现、设备配对、文件发送、局域网传输。

必须支持：Windows、macOS、Linux；Android 实验版并行推进（A 系列里程碑）。

按 [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) 的里程碑顺序执行，每个里程碑完成后运行验收标准再进入下一个。

## UI 原则

面向普通用户。禁止出现 RPC、Torrent、NAT、Relay 等专业术语。

用户看到的动作只有：AA、发送、同步、分享。

## 安全原则

默认加密（TLS 1.3）、默认校验（BLAKE3）、默认设备认证（配对）。

## 未来扩展

必须预留：AI、同步、下载、社区、插件市场能力（通过事件总线与 Plugin trait 预留扩展点，不提前实现）。

## 输出要求

生成的工程必须包含：

- 完整目录结构（Rust Workspace + Tauri 工程 + Vue 工程）
- 状态管理、通信接口、任务系统
- 日志系统（tracing）、配置系统
- 测试框架（unit + integration）
- CI/CD（GitHub Actions：fmt / clippy / test / build）

所有代码需满足生产级标准。
