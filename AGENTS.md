# AA4C Agent Development Rules

你是 AA4C 项目的开发 Agent。你的目标：构建一个长期维护的开源项目。

## 必读文档

开始任何编码任务前，先阅读：

1. [PROJECT_VISION.md](PROJECT_VISION.md) —— 产品定位与边界
2. [ARCHITECTURE.md](ARCHITECTURE.md) —— 分层架构与 crate 划分
3. [API_DESIGN.md](API_DESIGN.md) —— 模块接口契约（不得随意更改公共接口）
4. [PROTOCOL.md](PROTOCOL.md) —— 传输协议规范（协议变更必须保持版本兼容）
5. [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) —— 数据库表结构
6. [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md) —— 当前阶段的实现步骤
7. [TESTING.md](TESTING.md) —— 测试规范与必测项

## 产品原则

AA连接（AA4C）**不是**：下载器、BT 工具、云盘、网盘、同步工具、社区平台。

AA连接（AA4C）**是**：一个开源的跨平台**设备连接平台**。核心是"设备 → 连接 → 能力"——文件传输只是连接后的第一种能力。

## 开发优先级

永远遵循：

1. 稳定性 > 功能数量
2. 简单 > 复杂
3. 用户体验 > 技术炫耀

## 技术栈

| 层 | 技术 |
|----|------|
| Backend | Rust（edition 2021，async = tokio） |
| Desktop | Tauri 2 + Vue3 + TypeScript + Pinia |
| Mobile | Tauri 2 Android（与桌面端同一工程 `apps/desktop`，Android 原生部分用 Kotlin 插件） |
| Database | SQLite（rusqlite） |

## 架构规则

**必须**：

- 模块化：按 [ARCHITECTURE.md](ARCHITECTURE.md) 的 crate 划分写代码
- 插件化：高级能力通过 Plugin trait 接入
- 低耦合：服务之间只通过事件总线和公共类型通信

**禁止**：

- 巨型文件（单文件超过 ~500 行应考虑拆分）
- 循环依赖（crate 依赖必须是单向的：types ← 各服务 ← core）
- 业务混杂（一个 Service 只做一件事）

## Core 规则

Core 只负责：状态管理、生命周期、插件管理、配置。

Core **不实现**文件传输等任何业务细节。

## Service 规则

每个功能独立 Service：`TransferService`、`SyncService`、`DownloadService`、`AIService`。

Service 的公共接口必须与 [API_DESIGN.md](API_DESIGN.md) 保持一致；如需变更接口，先更新文档再改代码。

## Security 规则

- 所有网络通信必须加密（TLS 1.3）
- 所有文件必须校验哈希（BLAKE3）
- 所有设备必须认证（配对后才能传输）
- 私钥永远不离开设备，不写入日志

## UI 规则

面向普通用户。

**禁止在 UI 中暴露**：RPC、NAT、Torrent、STUN、TLS、mDNS 等专业术语。

**用户只看到**：AA、发送、同步、分享。

详细文案规范见 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md)。

## Git 规则

- 一个功能一个 commit
- commit 格式：`feat:` / `fix:` / `refactor:` / `docs:` / `test:` / `chore:`

## Testing 规则

核心模块必须有 unit test；跨模块流程（配对、传输）必须有 integration test。

提交前必须通过：

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Documentation 规则

新增功能必须同步更新：README、API_DESIGN、CHANGELOG。

## 不允许

- 复制 GPL 代码（GPL 组件仅通过 API / RPC 调用）
- 硬编码路径（使用 `dirs` crate 获取平台目录）
- 硬编码平台逻辑（用 `cfg` 或抽象层隔离）
- 依赖闭源服务

## 最终目标

创建一个人人可以使用的开源跨平台设备连接平台——连接你的所有设备。
