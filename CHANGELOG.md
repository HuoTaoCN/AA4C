# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式与[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Changed

- 产品定位升级为「**AA连接（AA4C）—— 开源跨平台设备连接平台**」（设备 → 连接 → 能力，连接优先）：明确不做社区/资源平台/中心化云盘；Slogan 改为"连接你的所有设备"；新增"连接优先"五阶段路线（AA Nearby → Sync → Connect → Touch → Direct）与 D2D 未来方向；移动端确认沿用 Tauri 2（iOS/iPad/平板由同一构建+响应式覆盖，Flutter 仅远期备选）。同步更新 README / PROJECT_VISION / ROADMAP / ARCHITECTURE / AGENTS / CONTRIBUTING / UI_DESIGN_SPEC / CODEX_MASTER_PROMPT。

### Added

- V0.2 设计文档：新增 [SYNC_DESIGN.md](SYNC_DESIGN.md) —— 设备信任分级（完全信任/朋友/临时/陌生）、跨设备文件索引、文件状态可视化（🟢 本地有 / 🟡 可下载 / 🔴 设备离线）、元数据优先+按需获取、Inbox「收到的」纳入索引；同步更新 PROJECT_VISION（权限分级 + 同步）、DATABASE_SCHEMA（V0.2 表：`devices.trust_level` / `sync_scopes` / `sync_file_index` / `remote_index` / `sync_conflicts`）、UI_DESIGN_SPEC（同步页统一文件视图 + 设置页信任层级）、ROADMAP。仅设计，未实现。
- 前端能力架构：导航围绕五大能力（传输/同步/分享/下载/归档）重构，首页能力卡片 + 建设中页 + PC 侧栏/移动底栏两套外壳；界面品牌改为「AA连接」。
- UI 设计预览（示例数据，后端 V0.2 接）：同步页跨设备文件的绿/黄/红状态视图（本地有 / 可下载 / 设备离线）+ 图例 + 筛选 + Inbox 分组；设置页「我的设备 ⇄ 朋友」信任分级分段切换。

## [0.1.1] - 2026-06-14

### Fixed

- 设备发现地址选择：`enable_addr_auto` 会广播对端所有网卡地址，其中可能混入代理虚拟网卡的不可达地址（典型为 Clash/代理 TUN 默认 fake-ip 段 `198.18.0.0/16`）。改为按可达性打分，优先私有 LAN IPv4，排除回环 / 链路本地 / `198.18.0.0/15` / `100.64.0.0/10`——修复**开着代理的电脑无法被对端（如 Android）发起配对/传输**的问题
- 默认设备名：去掉 hostname 的 `.local` 等 mDNS 后缀；hostname 缺失或为 `localhost`（Android 常见）时回落到平台名（Mac / Windows 电脑 / Android 手机 等），不再显示 `localhost` / `xxx.local`

## [0.1.0] - 2026-06-13

首个版本：第一次 AA —— 局域网内设备发现、配对、加密文件传输。桌面三平台 + Android 实验版。

### Added

- 项目文档体系：愿景白皮书、架构设计、API 设计、协议规范（ATP v1 + v2 草案）、数据库设计、UI 设计规范、V0.1 实现计划、测试指南、贡献指南、安全策略
- M0 工程脚手架：Cargo workspace（6 个 crate）、Tauri 2 + Vue3 + TypeScript 桌面端工程、tracing 日志、GitHub Actions CI（三平台 fmt / clippy / test / 前端构建 / cargo-audit）与 Release 工作流
- M1 类型与存储：`aa4c-types` 全部公共类型（设备 / 任务 / 事件 / 错误，API_DESIGN §3）；`aa4c-store` SQLite 持久化（user_version 迁移、专职线程 async 封装、设备 / 任务 / 设置 CRUD、外键级联）
- M2 设备身份：`aa4c-identity` —— Ed25519 密钥生成与持久化（0600）、rcgen 自签名证书、rustls TLS 1.3 mTLS 证书固定（双向指纹校验，正反向测试）、配对 PIN 推导（PROTOCOL §6.1）
- M3 设备发现：`aa4c-discovery` —— mDNS 注册（`_aa4c._tcp.local.` + TXT id/name/platform/ver/proto）与浏览、自身过滤、设备上线/更新/下线事件、真实组播双实例测试（#[ignore]，本地验证通过）
- M4 配对协议：新增 `aa4c-proto`（ATP v1 Message 定义、帧编解码、超长帧/截断防御、Hello 握手协商）；`PairingManager` 状态机（双向 PIN、声明公钥与 TLS 证书一致性校验、60s 超时、成功写库 trusted=1），4 个端到端测试（成功/拒绝请求/PIN 拒绝/超时）
- M5 传输引擎：`aa4c-transfer` —— TLS 监听 + 握手 trusted 校验、文件/文件夹流式收发（4 MiB 分块、BLAKE3 边传边校验）、路径净化（拒绝穿越/绝对路径）、重名自动加后缀、`.aa4c-part` 临时落盘、进度节流事件、取消与断连处理、哈希失败重传（≤2 次，放弃时发 Cancel 通知对端，符合 PROTOCOL §7）；8 个集成测试（单文件/空文件/深层目录+重名/中等文件/拒绝/取消/断连/未配对拒绝）+ 1GB 大文件测试（ignored）
- A0 Android 工程：`tauri android init` 生成 Android 工程（minSdk 24，com.aa4c.desktop），本地 aarch64 debug/release APK 构建通过；CI 新增 android 编译哨兵 job（不阻塞合并）
- M6 Core 组装 + Tauri 桥：`aa4c-core` 装配五大组件（identity / store / discovery / transfer / pairing）并以 broadcast 事件总线串联；启动序列含遗留任务清理（waiting_accept / transferring → failed）；统一监听端口分流（`Offer` 走传输、`PairRequest` 经 `IncomingPairDispatch` 钩子转交配对，传输层不感知配对语义）；`Settings` 类型 + 设置读写（device_name 变更重新广播 mDNS）；Tauri 层实现 API_DESIGN §9 全部 11 个 Command（`{ code, message }` 错误映射）与 `CoreEvent → aa4c://` 事件转发（扁平 camelCase payload）；2 个端到端冒烟测试（双 Core 配对+传输、重启清理遗留任务）
- A1 Android 平台适配：`MainActivity` 持有 / 释放 `WifiManager.MulticastLock`（Android 默认过滤组播，mDNS 发现必需）；`AndroidManifest` 增加 `ACCESS_NETWORK_STATE` / `CHANGE_WIFI_MULTICAST_STATE` / `POST_NOTIFICATIONS` 权限；接收目录改由 Tauri path resolver 注入（桌面=下载目录、Android 回落到应用可写目录），Core 以注入值为缺省、用户设置覆盖（API_DESIGN §11）；CI android 哨兵对齐到 `platforms;android-36`（compileSdk 36 所需）+ `build-tools;35.0.0`；aarch64 debug APK 本地构建通过
- M7 前端 UI：Vue3 + Vue Router + Pinia 桌面前端，4 个页面（首页 / AA 发送 / 记录 / 设置）+ 配对/接收弹窗 + 全局任务条 + toast；4 个 store（设备/配对/传输/设置）由 `aa4c://` 事件驱动，根组件统一监听；AA 页支持窗口拖拽（`onDragDropEvent`）与系统文件选择器（tauri-plugin-dialog），三步发送流；配对双向 PIN 大号确认码弹窗；接收确认弹窗可改保存目录；完成时系统通知（tauri-plugin-notification）+ toast；记录页分组（今天/昨天/更早）+ 打开文件夹（tauri-plugin-opener）；响应式 < 700px 切底部导航；深色模式跟随系统；全文案遵循 UI_DESIGN_SPEC §7 术语表（零技术词）；`pnpm build`（vue-tsc + vite）无类型错误

### Changed

- 移动端技术方案：Flutter → **Tauri 2 Android**（与桌面端共享同一工程与前端；Flutter 退为远期备选），Android 实验版纳入 V0.1 并行开发（A0–A3 里程碑）

### Planned (V0.1)

- 设备发现（mDNS）
- 设备配对（双向 PIN 确认）
- 局域网加密文件传输（TLS 1.3 + BLAKE3 校验）
- 桌面端（Tauri + Vue3：Windows / macOS / Linux）
- Android 实验版（Tauri 2，与桌面端同一代码库）

[Unreleased]: https://github.com/HuoTaoCN/AA4C/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/HuoTaoCN/AA4C/releases/tag/v0.1.1
[0.1.0]: https://github.com/HuoTaoCN/AA4C/releases/tag/v0.1.0
