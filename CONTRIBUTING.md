# Contributing to AA4C

感谢你对 AA4C 的兴趣！AA4C 的目标是构建一个长期维护的开源个人数字空间，欢迎任何形式的贡献：代码、文档、测试、翻译、Issue 反馈、使用体验建议。

## 开始之前

请先阅读：

1. [PROJECT_VISION.md](PROJECT_VISION.md) —— 理解 AA4C 是什么、不是什么
2. [ARCHITECTURE.md](ARCHITECTURE.md) —— 分层架构与 crate 划分
3. [API_DESIGN.md](API_DESIGN.md) —— 接口契约（**改接口必须先改文档**）

## 开发环境

| 工具 | 版本 |
|------|------|
| Rust | stable（最新），含 `rustfmt` + `clippy` |
| Node.js | ≥ 20 |
| pnpm | ≥ 9 |
| Tauri CLI | 2.x |

仅参与 Android 开发时额外需要：

| 工具 | 版本 |
|------|------|
| JDK | 17 |
| Android SDK | platform-tools、platforms;android-34、build-tools;34.0.0 |
| Android NDK | 27.x |
| Rust targets | `aarch64-linux-android` `armv7-linux-androideabi` `i686-linux-android` `x86_64-linux-android` |

环境变量：`JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME`。

各平台 Tauri 系统依赖见 [Tauri 官方文档](https://tauri.app/start/prerequisites/)（Linux 需要 webkit2gtk 等）。

```bash
git clone https://github.com/HuoTaoCN/AA4C.git
cd AA4C
cargo build                    # 构建全部 Rust crates
cd apps/desktop && pnpm install
pnpm tauri dev                 # 启动桌面端开发模式
```

## 贡献流程

1. **先开 Issue 讨论**（bug 报告可直接提；新功能 / 接口变更必须先讨论达成一致）
2. Fork 仓库，从 `main` 切出分支：`feat/xxx`、`fix/xxx`、`docs/xxx`
3. 开发，确保本地检查通过（见下）
4. 提交 PR，关联 Issue，描述清楚"做了什么、为什么、怎么验证"

## 提交前自检

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cd apps/desktop && pnpm build   # TS 类型检查 + 前端构建
```

CI 会在 Windows / macOS / Linux 三平台跑同样的检查，任一平台失败不予合并。

## Commit 规范

一个功能一个 commit，格式：

```
<type>: <一句话描述>

type: feat | fix | refactor | docs | test | chore | perf
```

示例：`feat: 实现 mDNS 设备发现服务`、`fix: 修复 Windows 路径分隔符导致的 rel_path 校验失败`。

## 代码规则

- 遵守 [AGENTS.md](AGENTS.md) 中的全部架构规则（crate 单向依赖、Core 不写业务、禁止巨型文件）
- 公共接口变更：**先改 [API_DESIGN.md](API_DESIGN.md)，再改代码**
- 数据库变更：迁移文件只追加不修改，同步更新 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)
- 协议变更：必须更新 [PROTOCOL.md](PROTOCOL.md) 并考虑版本兼容
- UI 文案：禁止专业术语，遵守 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) §7 术语表
- 测试：新功能必须带测试，规范见 [TESTING.md](TESTING.md)

## 许可证相关（重要）

- 项目协议为 **Apache-2.0**，你提交的代码默认按 Apache-2.0 授权
- **禁止复制 GPL / AGPL 代码**进仓库；GPL 组件（Aria2、qBittorrent 等）只允许通过 RPC / API 调用
- 引入新依赖时确认其协议为 MIT / Apache-2.0 / BSD 等宽松协议

## 安全问题

**不要**为安全漏洞开公开 Issue，请按 [SECURITY.md](SECURITY.md) 的流程私下报告。

## 行为准则

参与本项目即表示你同意遵守 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)。

## 提问与交流

- Bug / 功能建议：[GitHub Issues](https://github.com/HuoTaoCN/AA4C/issues)
- 设计讨论：[GitHub Discussions](https://github.com/HuoTaoCN/AA4C/discussions)
