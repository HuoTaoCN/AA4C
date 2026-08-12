# AA连接（AA4C）

**连接你的所有设备。 —— Connect all your devices.**

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Android-lightgrey.svg)](#平台支持)
[![Release](https://img.shields.io/badge/release-v0.7.0--preview.1-green.svg)](https://github.com/HuoTaoCN/AA4C/releases)

> 中文 · [English](README.en.md)

AA连接（AA4C = **AA for Connection**）是一个开源、跨平台的**设备连接平台**。先把你的所有设备连起来，文件传输、同步、分享、下载、AI 归档等能力都在连接之上自然发生——核心永远是**设备与设备的连接**。

> 把这个文件 AA 给我。
> 把照片 AA 到电脑。
> 把模型 AA 到服务器。

AA连接**不是**下载器、BT 工具、云盘、网盘、同步工具，也不是社区平台。它坚持一件事：**用户数据属于用户自己**。

---

## 目录

- [为什么是 AA4C](#为什么是-aa4c)
- [特色功能](#特色功能)
- [五大能力](#五大能力)
- [平台支持](#平台支持)
- [安装与上手](#安装与上手)
- [开源 · 开放 · 安全](#开源--开放--安全)
- [文档导航](#文档导航)
- [从源码构建](#从源码构建)
- [项目状态与路线](#项目状态与路线)
- [参与贡献](#参与贡献)

---

## 为什么是 AA4C

传统方式——数据要绕一圈第三方：

```
手机 → 云盘（第三方服务器） → 电脑
```

AA4C——设备直连，数据始终是你的：

```
手机 ↔ 电脑 ↔ NAS ↔ 服务器
```

| 原则 | 含义 |
|------|------|
| **点对点** | 设备之间直接传输，不经过任何第三方服务器 |
| **本地优先** | 数据保存在自己的设备上，不强制上传云端 |
| **默认加密** | 所有通信 TLS 1.3 端到端加密，加密不可关闭 |
| **用户控制** | 无账号、无云、无订阅；不配置就完全不出网 |

### 为什么叫 AA？

AA 是一个动作，代表 **Send（发送）· Sync（同步）· Share（分享）· Archive（归档）**。它是连接设备后的核心动作，我们希望它成为设备世界里的一个新动词。

**4C = for Connection**（4≈for、C=Connection）——一切围绕"连接"展开：**连接优先，功能其次**。

---

## 特色功能

这些是 AA4C 与同类工具真正不同的地方。

### 🔌 一个 App，覆盖五种能力

通常你需要 LocalSend（传文件）+ Syncthing（同步）+ Motrix/qBittorrent（下载）+ 手动整理文件。AA4C 把**传输、同步、分享、下载、归档**放进同一个应用、同一套设备身份、同一个任务中心——文件下载完可以自动归档，归档后自动同步到 NAS，全程不用换软件。

### 🌐 不配置就完全不出网

远程连接开关**默认关闭**。不开它，AA4C 只在局域网活动，不会向互联网发起任何连接。没有遥测、没有崩溃上报、没有"匿名统计"、没有账号体系——代码里搜不到任何分析 SDK 或第三方端点。

### 🏠 中继与信令**只允许自建**

跨网络连接需要信令/中继服务器时，AA4C **不提供官方公益节点**，只支持你自己部署 `aa4c-server`（单个二进制或 Docker）。这意味着链路完全在你手里，不存在"哪天官方服务器关停/被要求交数据"的问题。服务器只见密文与端点映射，看不到文件内容。

### 🟢 元数据优先的同步：不再被磁盘拖垮

传统同步工具会把整个文件夹在每台设备上落一份。AA4C 默认**只同步文件名与目录结构**，内容点了才拉。每个文件按可获取性着色：

| 状态 | 含义 |
|------|------|
| 🟢 本地有 | 内容已在本机，直接打开 |
| 🟡 可下载 | 内容在某台在线设备上，点一下就取回 |
| 🔴 设备离线 | 只有离线设备有，暂时取不到 |

于是 1 TB 的素材库可以完整"出现"在轻薄本上，只占几 MB 索引。

### 🛡️ 设备信任分级，而不是"配对了就全给"

配对成功 ≠ 交出全部文件。AA4C 把信任分成四级，只有你亲手升级为「完全信任」的**自己的设备**才参与跨设备索引与同步：

| 层级 | 典型对象 | 跨设备索引 / 同步 | 收文件 |
|------|----------|-------------------|--------|
| 完全信任 | 自己的多台设备 | ✅ 双向 | 可设为免确认 |
| 朋友 | 朋友 / 家庭 / 团队 | ❌ 仅手动分享 | 需确认 |
| 临时 | 单次收发对象 | ❌ | 当次需确认 |
| 陌生 | 已发现未配对 | ❌ | 拒收 |

配对默认落在「朋友」——**默认最小权限**。

### 🤖 AI 完全本地，且"规则自动、AI 建议"

AI 归档跑在本机的 llama.cpp（`llama-server`）上，模型是你自己的 GGUF 文件，**零云端调用**，断网可用；引擎懒启动、空闲自停，不占着内存。

更关键的是权限边界：**只有确定性规则可以自动移动文件；AI 的输出永远只进"待确认"队列，从不擅自动你的文件**。AI 猜错了，最坏的结果只是一条你可以忽略的建议。

### 📚 对自己的文件提问（本地知识库）

选一个目录作为知识库来源，AA4C 会在本地建索引；之后可以直接提问，答案带原文引用。文档不出本机、不进任何云服务。

### 🧠 认识模型文件的下载器

下载 `Qwen3-4B-Q4_K_M.gguf` 这类模型时，AA4C 会**手写解析 GGUF 文件头**（只读前几十 KB，永不读张量数据），识别架构、参数规模、量化等级和上下文长度，自动归档进模型库并在设备间同步。对本地跑模型的人来说，这是别的下载器不会做的事。

### ⚖️ 干净的许可证边界

aria2、Transmission 是 GPL 组件。AA4C **不复制它们的源码、不链接它们的库**——它们作为独立子进程运行，只通过**回环地址（127.0.0.1）+ 每次启动随机生成的密钥**做 RPC 调用。所以 AA4C 自身能保持 Apache-2.0，企业可以放心集成。

### ↩️ 每一次自动操作都可撤销

自动归档移动过的文件，在「归档 → 最近动作」里一键撤销回原位。自动化必须是可逆的，否则就是在替用户做不可挽回的决定。

---

## 五大能力

| 能力 | 说明 | 状态 |
|------|------|------|
| **AA Send** 传输 | 局域网设备自动发现 + 点对点加密直传，支持文件 / 文件夹 / 大文件，断点续传 | ✅ V0.1 |
| **AA Sync** 同步 | 多设备文件夹同步：元数据优先、按需获取、实时监听、冲突并列保留 | ✅ V0.2 |
| **AA Share** 分享 | 广域网连接（QUIC + NAT 打洞 + 自建中继）与分享链接，给指定好友 / 家庭 / 团队 | ✅ V0.3 |
| **AA Download** 下载 | 统一下载中心：HTTP / HTTPS / FTP（aria2）+ BT / 磁力（Transmission） | ✅ V0.4 |
| **AA Archive & AI** 归档 | 规则式自动分类归档、模型库、AI 标签建议、本地知识库问答，全程零云端 | ✅ V0.5 |

各能力的详细用法见 [用户手册](docs/USER_GUIDE.md)。

---

## 平台支持

| 类型 | 系统 | 技术 | 状态 |
|------|------|------|------|
| 桌面 | Windows / macOS / Linux | Tauri 2 + Vue 3 | ✅ 全能力可用 |
| 移动 | Android | Tauri 2（与桌面同一代码库） | 🧪 实验版，覆盖传输 / 同步 |
| 移动 | iOS / iPad | Tauri 2（同一代码库） | 📋 计划中 |
| 服务器 | Linux x86_64（NAS / VPS） | `aa4c-server` 单二进制 | ✅ 信令 + 中继 |

> 下载中心与 AI 归档目前仅桌面三平台。

---

## 安装与上手

### 下载安装包（推荐）

从 [Releases](https://github.com/HuoTaoCN/AA4C/releases) 下载对应平台的安装包：

| 平台 | 文件 | 说明 |
|------|------|------|
| macOS | `.dmg` | 通用包（Apple Silicon + Intel）。未签名，首次打开请右键「打开」，或在「系统设置 → 隐私与安全性」中放行 |
| Windows | `.msi` | 双击安装 |
| Linux | `.deb` / `.rpm` / `.AppImage` | AppImage 需 `chmod +x` 后直接运行 |
| Android（实验版） | `.apk` | 需在系统设置中允许「安装未知来源应用」 |

### 三步完成第一次 AA

1. 两台设备连接**同一个 WiFi / 局域网**，都打开 AA4C——首页会自动出现对方
2. 点对方卡片「配对」，**两台屏幕核对 6 位确认码一致**后确认
3. 进「传输」页：选文件 → 选设备 → 点 AA

想让两台设备互相同步文件，在配对成功的弹窗里选「是，我的设备」，把对方升级为**完全信任**即可。

### 找不到设备？先查防火墙

AA4C 用到这些端口：

| 端口 | 用途 | 什么时候用到 |
|---|---|---|
| **TCP 42420** | 文件传输、配对 | 一直 |
| **UDP 42420** | QUIC（跨网传输走这条） | 一直 |
| **UDP 5353** | mDNS 设备发现 | 一直 |
| **TCP + UDP 42421** | 内置服务器（V0.7 起） | **只在你打开「让这台设备当中转站」时** |

- **Windows**：首次运行会弹防火墙询问，务必勾选 **「专用网络」**
- **macOS**：首次运行弹「是否允许接受传入连接」，点**允许**
- **Linux（ufw）**：`sudo ufw allow 42420/tcp && sudo ufw allow 42420/udp && sudo ufw allow 5353/udp`
  （开了内置服务器再加 `sudo ufw allow 42421/tcp && sudo ufw allow 42421/udp`）
- 部分公司网络 / 公共 WiFi 会开启「客户端隔离」或屏蔽组播，设备之间根本无法互相看到——换一个普通家用 WiFi 重试

更多排查见 [常见问题](docs/FAQ.md)。

---

## 开源 · 开放 · 安全

这三条不是口号，都可以在代码里逐条核对。详细说明见 [《开源 · 开放 · 安全》](docs/OPEN_AND_SECURE.md)。

### 开源

- **Apache License 2.0**——允许商业使用、允许企业集成、提供专利授权
- 全部代码、构建脚本、CI 配置、设计文档公开；每个版本由 GitHub Actions 在三平台公开构建
- 依赖只用 MIT / Apache-2.0 / BSD 等宽松协议；GPL 组件严格以子进程 + RPC 隔离，不污染许可证

### 开放

- **协议公开**：线路协议、帧格式、握手与配对流程全部写在 [PROTOCOL.md](PROTOCOL.md)，任何人都可以实现兼容客户端
- **数据可带走**：元数据存本地 SQLite（[DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)），文件就是普通文件——没有专有容器、没有加密牢笼，卸载 AA4C 不会带走你的任何数据
- **基础设施可自建**：信令与中继服务器只支持自部署，[自建指南](docs/SELF_HOSTING.md)
- **面向扩展**：分层的 Plugin API 与开放 API 是 V1.0 的既定目标（[ROADMAP.md](ROADMAP.md)）

### 安全

| 层 | 机制 |
|----|------|
| 设备身份 | Ed25519 密钥对；DeviceId = BLAKE3(公钥)，不可伪造 |
| 信任建立 | 双向 6 位 PIN 目视确认；**PIN 两端独立算出、从不经过网络** |
| 通道加密 | TLS 1.3 + 自签证书 + 证书固定（指纹 = DeviceId），不依赖任何 CA |
| 数据完整性 | 文件级 BLAKE3 校验，失败自动重传 |
| 授权 | 只有已配对设备可发起传输；接收默认需确认；四级信任分级 |
| 密钥保管 | 私钥仅存本地（文件权限 0600），永不入库、不入日志、不离开设备 |
| 本地引擎 | aria2 / Transmission / llama-server 一律绑定 127.0.0.1 + 随机密钥，不对局域网暴露 |

**安全特性不可配置关闭**——项目规则明令禁止引入"关闭加密/校验"的开关。

完整威胁模型与漏洞报告流程见 [SECURITY.md](SECURITY.md)。

---

## 文档导航

### 给使用者

| 文档 | 内容 |
|------|------|
| 📖 [用户手册](docs/USER_GUIDE.md) | 逐个功能的完整使用说明（传输 / 同步 / 分享 / 下载 / 归档 / 设置） |
| ❓ [常见问题与故障排查](docs/FAQ.md) | 找不到设备、传输失败、下载卡住、AI 用不了…… |
| 🖥️ [自建服务器指南](docs/SELF_HOSTING.md) | 部署 `aa4c-server`，打通跨网络连接 |
| 🔓 [开源 · 开放 · 安全](docs/OPEN_AND_SECURE.md) | 隐私承诺、数据去向、许可证边界、安全模型 |

> English documentation: [docs/en/](docs/en/)

### 给开发者

| 文档 | 内容 |
|------|------|
| [PROJECT_VISION.md](PROJECT_VISION.md) | 产品需求与技术架构白皮书 |
| [ARCHITECTURE.md](ARCHITECTURE.md) | 总体架构与 crate 划分 |
| [API_DESIGN.md](API_DESIGN.md) | Rust 模块接口设计 |
| [PROTOCOL.md](PROTOCOL.md) | AA 传输协议规范（局域网 + 广域网） |
| [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) | SQLite 数据库设计 |
| [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) | UI 与交互设计规范 |
| [TESTING.md](TESTING.md) | 测试策略与验收清单 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 贡献指南 |
| [SECURITY.md](SECURITY.md) | 安全策略与威胁模型 |
| [AGENTS.md](AGENTS.md) | AI Agent 开发规则 |
| [HANDOFF.md](HANDOFF.md) | 开发交接：环境、踩坑、当前进度 |
| [CHANGELOG.md](CHANGELOG.md) | 变更日志 |

### 模块详细设计

| 文档 | 对应能力 |
|------|----------|
| [SYNC_DESIGN.md](SYNC_DESIGN.md) | AA Sync：信任分级、跨设备索引、按需获取、冲突处理 |
| [CONNECT_DESIGN.md](CONNECT_DESIGN.md) | AA Connect / Share：信令、中继、NAT 打洞、分享链接 |
| [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) | AA Download：引擎集成、任务模型、限速与合规 |
| [ARCHIVE_DESIGN.md](ARCHIVE_DESIGN.md) | AA Archive & AI：规则引擎、GGUF 解析、AI 建议、知识库 |
| [TOUCH_DESIGN.md](TOUCH_DESIGN.md) | AA Touch / Direct：NFC 碰一碰、WiFi Direct、蓝牙（设计稿） |

---

## 从源码构建

环境要求：Rust stable（≥ 1.85）、Node.js ≥ 20、pnpm ≥ 9、Tauri CLI 2.x。各平台系统依赖见 [Tauri 官方文档](https://tauri.app/start/prerequisites/)。

```bash
git clone https://github.com/HuoTaoCN/AA4C.git
cd AA4C
cargo test --workspace              # Rust 全量测试
cd apps/desktop && pnpm install
pnpm test                           # 前端单元测试
pnpm tauri dev                      # 启动桌面端开发模式
pnpm tauri build                    # 打本平台安装包
```

提交前自检（CI 在三平台跑同样的检查）：

```bash
cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

环境安装细节与已知踩坑见 [HANDOFF.md](HANDOFF.md)，开发规范见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 项目状态与路线

**当前版本：v0.7.0-preview.1** —— V0.1 至 V0.5 与 V0.7 的能力均已实现。V0.6（碰一碰 / 脱网连接）
设计定稿但尚未实现，它依赖的 NFC 与 WiFi Direct 能力**仅 Android 具备**（见 TOUCH_DESIGN.md §1.1）。

| 版本 | 代号 | 目标 | 状态 |
|------|------|------|------|
| V0.1 | Alpha | 局域网发现、配对、文件传输 | ✅ 已发布 |
| V0.2 | Beta | 信任分级、跨设备索引、持续同步 | ✅ 已发布 |
| V0.3 | Connect | NAT 穿透、自建中继、分享链接 | ✅ 已发布 |
| V0.4 | Download | 统一下载中心（HTTP/FTP + BT/磁力） | ✅ 已发布 |
| V0.5 | AI | 规则归档、模型库、AI 建议、本地知识库 | ✅ 已发布（预览） |
| V0.6 | Touch / Direct | 碰一碰配对（NFC）、脱网连接（WiFi Direct / 蓝牙） | 📐 设计定稿，待实现 |
| V0.7 | Trust / Reach | 信任传递（引荐确认）、IPv6 双栈、UPnP、内置服务器 | ⚠️ 已实现，**跨网部分待真机验证** |
| V1.0 | Ecosystem | 全平台 + 插件系统 + 开发者 SDK | 📋 规划中 |

> **V0.7 为什么标「待验证」而不是「已发布」**：四个里程碑代码都完成、自动化测试也全绿，但它们
> 真正要证明的三件事——**跨网直连、UPnP 真的在路由器上开了端口、内置服务器能被外网找到**——
> 在开发机上验不了（真实 UPnP 会改开发者自己路由器的配置；跨网需要两个真实的不同网络）。
> 想帮忙验证的话见 [docs/V0.7_VERIFICATION.md](docs/V0.7_VERIFICATION.md)。

详细排期见 [ROADMAP.md](ROADMAP.md)。

### 明确不做

| 不做 | 原因 |
|------|------|
| 社区 / 内容平台 | 内容审核与法律风险极高，非本项目目标 |
| 资源广场 / 模型广场 / 文件社区 | 不做中心化内容分发 |
| 中心化云盘 / 网盘 | 坚持用户数据属于用户自己，不建集中存储 |
| 官方公益中继节点 | 基础设施只允许自建，避免单点与数据风险 |

---

## 参与贡献

欢迎代码、文档、测试、翻译、Issue 反馈与使用体验建议。

- 开始之前请读 [CONTRIBUTING.md](CONTRIBUTING.md) 与 [PROJECT_VISION.md](PROJECT_VISION.md)（理解 AA4C 是什么、不是什么）
- Bug 与功能建议：[GitHub Issues](https://github.com/HuoTaoCN/AA4C/issues)
- 设计讨论：[GitHub Discussions](https://github.com/HuoTaoCN/AA4C/discussions)
- **安全漏洞请勿开公开 Issue**，按 [SECURITY.md](SECURITY.md) 私下报告
- 参与本项目即表示同意遵守 [行为准则](CODE_OF_CONDUCT.md)

## 许可证

[Apache License 2.0](LICENSE)

## 社区

- GitHub: https://github.com/HuoTaoCN/AA4C
- 官网: https://aa4c.com
