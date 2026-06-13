# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式与[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- 项目文档体系：愿景白皮书、架构设计、API 设计、协议规范（ATP v1 + v2 草案）、数据库设计、UI 设计规范、V0.1 实现计划、测试指南、贡献指南、安全策略
- M0 工程脚手架：Cargo workspace（6 个 crate）、Tauri 2 + Vue3 + TypeScript 桌面端工程、tracing 日志、GitHub Actions CI（三平台 fmt / clippy / test / 前端构建 / cargo-audit）与 Release 工作流
- M1 类型与存储：`aa4c-types` 全部公共类型（设备 / 任务 / 事件 / 错误，API_DESIGN §3）；`aa4c-store` SQLite 持久化（user_version 迁移、专职线程 async 封装、设备 / 任务 / 设置 CRUD、外键级联）
- M2 设备身份：`aa4c-identity` —— Ed25519 密钥生成与持久化（0600）、rcgen 自签名证书、rustls TLS 1.3 mTLS 证书固定（双向指纹校验，正反向测试）、配对 PIN 推导（PROTOCOL §6.1）
- M3 设备发现：`aa4c-discovery` —— mDNS 注册（`_aa4c._tcp.local.` + TXT id/name/platform/ver/proto）与浏览、自身过滤、设备上线/更新/下线事件、真实组播双实例测试（#[ignore]，本地验证通过）
- M4 配对协议：新增 `aa4c-proto`（ATP v1 Message 定义、帧编解码、超长帧/截断防御、Hello 握手协商）；`PairingManager` 状态机（双向 PIN、声明公钥与 TLS 证书一致性校验、60s 超时、成功写库 trusted=1），4 个端到端测试（成功/拒绝请求/PIN 拒绝/超时）
- M5 传输引擎：`aa4c-transfer` —— TLS 监听 + 握手 trusted 校验、文件/文件夹流式收发（4 MiB 分块、BLAKE3 边传边校验）、路径净化（拒绝穿越/绝对路径）、重名自动加后缀、`.aa4c-part` 临时落盘、进度节流事件、取消与断连处理、哈希失败重传（≤2 次，放弃时发 Cancel 通知对端，符合 PROTOCOL §7）；8 个集成测试（单文件/空文件/深层目录+重名/中等文件/拒绝/取消/断连/未配对拒绝）+ 1GB 大文件测试（ignored）
- A0 Android 工程：`tauri android init` 生成 Android 工程（minSdk 24，com.aa4c.desktop），本地 aarch64 debug/release APK 构建通过；CI 新增 android 编译哨兵 job（不阻塞合并）

### Changed

- 移动端技术方案：Flutter → **Tauri 2 Android**（与桌面端共享同一工程与前端；Flutter 退为远期备选），Android 实验版纳入 V0.1 并行开发（A0–A3 里程碑）

### Planned (V0.1)

- 设备发现（mDNS）
- 设备配对（双向 PIN 确认）
- 局域网加密文件传输（TLS 1.3 + BLAKE3 校验）
- 桌面端（Tauri + Vue3：Windows / macOS / Linux）
- Android 实验版（Tauri 2，与桌面端同一代码库）

[Unreleased]: https://github.com/HuoTaoCN/AA4C/commits/main
