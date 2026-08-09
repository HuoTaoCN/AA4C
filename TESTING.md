# AA4C Testing Guide

> 测试策略与规范。CI 在 Windows / macOS / Linux 三平台执行，全部通过才允许合并。

## 1. 测试金字塔

```
        ┌──────────────┐
        │  手动验收测试  │   每个版本发布前，真机跨平台（§5 清单）
        ├──────────────┤
        │  端到端测试    │   双 Core 实例：配对 → 传输 → 断言
        ├──────────────┤
        │  集成测试      │   crate 间协作（tests/ 目录）
        ├──────────────┤
        │  单元测试      │   纯逻辑：解析、状态机、路径净化、PIN 推导
        └──────────────┘
```

## 2. 运行方式

```bash
cargo test --workspace                 # 全部 Rust 测试
cargo test -p aa4c-transfer            # 单 crate
cargo test -p aa4c-transfer --test e2e # 单个集成测试文件
cargo test -- --nocapture              # 显示 println/tracing 输出

cd apps/desktop
pnpm test                              # 前端单元测试（Vitest）
pnpm build                             # TS 类型检查兜底
```

提交前完整自检：

```bash
cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

## 3. 测试编写规范

### 通用规则

1. **端口一律用 0**（系统分配），禁止硬编码端口 —— 避免 CI 并发冲突
2. **数据目录一律用 `tempfile::TempDir`**，测试结束自动清理
3. 集成测试中起的双实例：各自独立 TempDir、独立端口
4. 涉及等待的断言用**轮询 + 超时**（如 `tokio::time::timeout(10s, …)`），禁止裸 `sleep` 后直接断言
5. 测试名描述行为：`rejects_unpaired_device`、`resumes_after_hash_mismatch`，而非 `test1`
6. 网络相关测试优先走 `127.0.0.1`；依赖真实组播（mDNS）的测试标记 `#[ignore]`，由本地与 nightly CI 跑

### 各 crate 必测项

| crate | 必测 |
|-------|------|
| `aa4c-types` | serde 序列化往返（事件、任务） |
| `aa4c-store` | 迁移幂等、CRUD 往返、外键级联删除 |
| `aa4c-identity` | device_id 稳定性、TLS 指纹匹配通过 / 不匹配拒绝（**正反两向**）、PIN 双端一致且与公钥顺序无关 |
| `aa4c-discovery` | TXT 解析、自身过滤；双实例发现/离线（ignore 标记） |
| `aa4c-transfer` | 帧编解码往返、超长帧拒绝、rel_path 路径穿越拒绝、单文件/空文件/深层目录/大文件哈希一致、拒绝/取消/断连/篡改重传 |
| `aa4c-core` | 启动→配对→传输→记录 的端到端冒烟测试 |

### 安全相关测试（强制）

凡涉及密钥、TLS 校验、路径处理的代码，PR 必须包含**正反两个方向**的测试：合法输入通过 + 非法输入被拒绝。只测 happy path 的安全代码不予合并。

## 4. CI 矩阵

`.github/workflows/ci.yml`：

| Job | 平台 | 内容 |
|-----|------|------|
| lint | ubuntu | `cargo fmt --check` + `clippy -D warnings` |
| test | ubuntu / macos / windows | `cargo test --workspace` |
| frontend | ubuntu | `pnpm install && pnpm test && pnpm build` |
| audit | ubuntu | `cargo audit`（依赖安全公告） |
| android | ubuntu | `tauri android build --debug`（编译哨兵，A0 起启用，不阻塞合并） |
| build | 三平台 | `tauri build`（仅 tag / release 分支） |

## 5. 手动验收清单（每个 release 前）

> **V0.7「Trust / Reach」另有一份专门的真机验证清单**：
> [docs/V0.7_VERIFICATION.md](docs/V0.7_VERIFICATION.md)。
> 那四个里程碑（IPv6 双栈 / 信任传递 / UPnP 端口映射 / 内置服务器）的核心主张**自动化测试
> 证明不了**——UPnP 的真实交互会改跑测试那台机器所在路由器的配置，跨网直连需要两个真实的
> 不同网络。下面这份是通用验收，那份是 V0.7 专项。

至少一组真实跨平台设备（如 macOS ↔ Windows）：

- [ ] 同一 WiFi 下 10 秒内互相发现
- [ ] 配对：PIN 两端一致，确认后状态变"已配对"
- [ ] 配对拒绝路径：一端拒绝，另一端有明确提示
- [ ] 发送单个小文件、发送含中文/空格/emoji 文件名的文件
- [ ] 发送 ≥1GB 大文件：进度与速度显示正常，完成后哈希一致（用 `b3sum` 抽查）
- [ ] 发送多层文件夹：目录结构保留
- [ ] 接收方拒绝：发送方收到"对方拒绝"
- [ ] 传输中取消（双向各试一次）：临时文件被清理
- [ ] 传输中断网/杀进程：10 秒内另一端报失败，不卡死
- [ ] 记录页：记录正确、"打开所在文件夹"可用、失败可重试
- [ ] 重名文件接收：自动加 `(1)` 后缀，不覆盖
- [ ] 深色模式、最小窗口 900×600 无横向滚动
- [ ] UI 全程无专业术语（按 UI_DESIGN_SPEC §7 术语表抽查）

**Android 追加项（A3 起）**：

- [ ] Android 真机 ↔ 桌面互相发现、配对、双向传输（哈希一致）
- [ ] 移动布局：底部导航、单列列表、无横向滚动、触控目标不误触
- [ ] 文件选择器可选多文件；接收后保存位置提示正确
- [ ] 锁屏/切后台中断后恢复前台：任务状态如实显示失败而非卡死
- [ ] Android 13+ 通知权限请求流程正常

## 6. 性能基准（参考值，不阻塞合并）

`benches/`（criterion）：

| 基准 | 目标 |
|------|------|
| 本机回环 1GB 传输 | ≥ 100 MB/s |
| BLAKE3 哈希吞吐 | ≥ 1 GB/s（依硬件） |
| 帧编解码 | ≥ 100k msg/s |

回归超过 20% 时在 PR 中说明原因。
