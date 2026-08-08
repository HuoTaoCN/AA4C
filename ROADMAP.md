# AA连接（AA4C）Roadmap

## Vision

连接你的所有设备。

> **连接优先，功能其次。** 能力围绕"连接方式"分阶段演进：
> AA Nearby（近场）→ AA Sync（同步）→ AA Connect（远程）→ AA Touch（碰一碰 / NFC）→ AA Direct（脱网 / WiFi Direct·蓝牙·蓝牙 Mesh）。详见 [PROJECT_VISION.md](PROJECT_VISION.md) 连接优先路线。

## 总览

| 版本 | 代号 | 连接阶段 | 目标 | 预计周期 |
|------|------|----------|------|----------|
| V0.1 | Alpha | AA Nearby | 完成第一次 AA（局域网） | 4 周 |
| V0.2 | Beta | AA Sync | 完成持续同步 + 信任分级 + 跨设备索引 | +4 周（累计 8 周） |
| V0.3 | Connect | AA Connect | 突破局域网（NAT 穿透 / P2P / Relay）+ 好友分享 | +4 周（累计 12 周） |
| V0.4 | Download | — | 统一文件入口（下载中心） | +4 周（累计 16 周） |
| V0.5 | AI | — | AI 归档（分类 / 标签 / 知识库） | +4 周（累计 20 周） |
| V0.6 | Touch/Direct | AA Touch / AA Direct | 碰一碰连接 / 脱网连接 | +4 周（累计 24 周） |
| V0.7 | Trust/Reach | — | 信任传递（引荐确认）+ IPv6 双栈 / 降低自建门槛 | +4 周（累计 28 周） |
| V1.0 | Ecosystem | — | 完整连接平台（桌面 + 移动/iPad + NAS + Docker + 插件） | 持续演进 |

---

## V0.1 — Alpha

**目标：完成第一次 AA。**

功能：

- 设备发现（mDNS）
- 设备配对（公钥 + PIN）
- 文件 / 文件夹发送
- 局域网加密传输
- 传输记录与任务管理

支持平台：Windows / macOS / Linux + **Android（实验版，Tauri 2 同一代码库，与桌面端并行开发）**

📄 详细实现步骤见 [V0.1_IMPLEMENTATION_PLAN.md](V0.1_IMPLEMENTATION_PLAN.md)。

---

## V0.2 — Beta

**目标：完成持续同步。**

功能：

- **设备信任分级**（完全信任 / 朋友 / 临时 / 陌生）
- **跨设备文件索引 + 状态可视化**（🟢 本地有 / 🟡 可下载 / 🔴 设备离线）
- **元数据优先 + 按需获取**（只同步文件名/目录，内容点了才拉）
- **Inbox「收到的」**纳入索引（在 A 收到、在 B 也能取）
- 文件夹同步（单向 / 双向）、自动同步（文件监听）、同步规则、冲突解决、同步历史

> 设计详见 [SYNC_DESIGN.md](SYNC_DESIGN.md)。

平台：Android 体验完善（前台服务、系统分享接入）；iOS（Tauri 2，视签名与审核情况）

---

## V0.3 — Connect

**目标：突破局域网。**

功能：

- NAT 穿透（STUN / 打洞）
- 设备中继（Relay，**仅自建**，不做官方公益节点）
- 远程同步、远程发送
- 分享链接

> 设计详见 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)；实现计划见 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)；线路层见 [PROTOCOL.md](PROTOCOL.md) Part B。

---

## V0.4 — Download

**目标：统一文件入口。**

功能：

- HTTP / HTTPS / FTP 下载（Aria2 RPC，桌面自动打包管理子进程）
- BT / Magnet 下载（Transmission RPC，D2 里程碑——v3 设计修订从 qBittorrent 换引擎）
- 统一任务中心
- Lua 插件系统预留（私有 Tracker/PT、搜索、自动分类等站点化需求，实现是 V0.4 之后的独立里程碑）

> 设计详见 [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md)（v3：D2 换 Transmission + Lua 插件预留边界）；实现计划见 [V0.4_IMPLEMENTATION_PLAN.md](V0.4_IMPLEMENTATION_PLAN.md)（D1–D3，D1 细化到步骤级）。**里程碑 D1（Aria2/HTTP-FTP）已实现**；D2（Transmission/BT-Magnet）、D3（任务中心打磨）仍是设计稿。V0.4 仅桌面三平台，不含 Android。

---

## V0.5 — AI

**目标：数字资产管理。**

功能：

- 自动分类、自动标签、自动归档（规则自动、AI 建议——AI 输出永不直接驱动文件操作）
- 模型管理（GGUF 识别与归档）
- 本地知识库（llama.cpp，完全本地、零云端调用）

> 设计详见 [ARCHIVE_DESIGN.md](ARCHIVE_DESIGN.md)（v1 定稿，关键外部事实已真机实证）；实现计划见 [V0.5_IMPLEMENTATION_PLAN.md](V0.5_IMPLEMENTATION_PLAN.md)。**里程碑 AI1–AI5 全部已实现，已随 `v0.5.0-preview` 打包发布**。V0.5 仅桌面三平台，不含 Android。

---

## V0.6 — Touch / Direct

**目标：更自然、更极端的连接方式。**

功能：

- **AA Touch（碰一碰）**：NFC —— 设备 A 碰设备 B → 自动配对（**仅 Android**，桌面三平台与 iOS 缺乏可用的第三方 NFC API，见 TOUCH_DESIGN.md §1.1）
- **AA Direct（脱网连接）**：WiFi Direct（**仅 Android**）+ 蓝牙（Android 双向对等，桌面仅扫描/接收）—— 没有互联网、没有基站也能连
- 面向 Device-to-Device（D2D）设备直连的演进（手机↔手机/电脑/NAS/车机/无人机）
- 蓝牙 Mesh 明确后置，不在本里程碑范围内（无操作系统级 API，见 TOUCH_DESIGN.md §1.4/§10）

> 设计详见 [TOUCH_DESIGN.md](TOUCH_DESIGN.md)（v1 设计稿，关键平台能力事实已用官方文档源核实——本环境无真实 NFC/WiFi Direct/蓝牙硬件，真机验证责任在用户）；实现计划见 [V0.6_IMPLEMENTATION_PLAN.md](V0.6_IMPLEMENTATION_PLAN.md)（里程碑 T1–T4）。不做社区 / 资源平台 / 中心化云盘——见 [PROJECT_VISION.md](PROJECT_VISION.md) 产品边界。

---

## V0.7 — Trust / Reach

**目标：让「我的几台设备」在不同网络下自己连成一片，不需要注册任何账号。**

V0.3 已经打通跨网连接的技术路径，但留下两个使用层面的门槛：要自建服务器；信任只能当面建立（配对走 PIN，要求同一局域网）。后者是死结——家里的台式机和单位的台式机永远不会同处一网，连得上也互不认识。

功能：

- **信任传递（引荐 + 一次确认）**：在家给手机和家里电脑配对，在单位给手机和单位电脑配对，之后手机把「这也是你的设备」的指纹引荐给彼此，用户各点一次确认即可互信——**不需要把台式机搬来搬去**。刻意不做自动信任（Syncthing 的 introducer 有传递失控与「删了又被加回来」两个已记录在案的坑）
- **IPv6 双栈**：现状是全链路只绑 IPv4，IPv6 基本不会被选中；而国内家宽普遍下发公网 IPv6、IPv4 反而在 CGNAT 后。打通后大量场景可直接落到「公网直连」，跳过打洞与中继
- **UPnP / NAT-PMP 自动端口映射**：目前只有 BT 端口做了，AA4C 自己的端口没做
- **桌面端内置可选 server 模式**：把「要有 VPS」降到「家里有台常开设备」
- 明确不做：虚拟网卡 / TUN 组网、公共节点池、全网 DHT、账号体系

> 设计详见 [TRUST_DESIGN.md](TRUST_DESIGN.md)（设计稿，未实现；里程碑 R1–R4）。三条前提已核实：Tailscale / ZeroTier **都需要账号或自建控制面**，真正无账号的是 Syncthing / EasyTier 那一类；「零第三方」做不到 NAT 穿透，目标应是「不依赖第三方**服务商**」；我们只要一条应用层连接，不需要照搬 VPN 式组网。

---

## V1.0 — Ecosystem

**目标：完整连接平台 + 开放生态。**

功能：

- 桌面 + 移动端（含 iOS / iPad / 平板）+ NAS + Docker 全平台
- 插件市场、开发者 SDK、开放 API
- 第三方扩展

持续演进。
