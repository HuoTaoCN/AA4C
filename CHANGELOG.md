# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式与[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- 项目文档体系：愿景白皮书、架构设计、API 设计、协议规范（ATP v1 + v2 草案）、数据库设计、UI 设计规范、V0.1 实现计划、测试指南、贡献指南、安全策略
- M0 工程脚手架：Cargo workspace（6 个 crate）、Tauri 2 + Vue3 + TypeScript 桌面端工程、tracing 日志、GitHub Actions CI（三平台 fmt / clippy / test / 前端构建 / cargo-audit）与 Release 工作流

### Planned (V0.1)

- 设备发现（mDNS）
- 设备配对（双向 PIN 确认）
- 局域网加密文件传输（TLS 1.3 + BLAKE3 校验）
- 桌面端（Tauri + Vue3：Windows / macOS / Linux）

[Unreleased]: https://github.com/HuoTaoCN/AA4C/commits/main
