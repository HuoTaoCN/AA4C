# AA4C 碰一碰与脱网连接设计（V0.6「Touch/Direct」）

> 状态：**设计稿 v1，未实现**。对应 [ROADMAP.md](ROADMAP.md) V0.6（AA Touch 碰一碰 / AA Direct 脱网连接）；实现拆解见 [V0.6_IMPLEMENTATION_PLAN.md](V0.6_IMPLEMENTATION_PLAN.md)。
>
> **本文档 §1 的平台能力事实已用官方文档源逐条核实**（不是凭记忆写的）：Android 经典 NFC P2P（Android Beam）已废弃/移除、官方 `tauri-plugin-nfc` 不支持 HCE、Android `WifiP2pManager` 现役、`btleplug` 只支持 central 角色、iOS HCE 对第三方 App 的限制。**本环境没有任何 NFC/WiFi Direct/蓝牙硬件，也没有真实 Android 设备**——本文档给出的是可验证来源的平台文档事实，不是本机实测；实现期每一步的真机验证责任在用户，见 [V0.6_IMPLEMENTATION_PLAN.md](V0.6_IMPLEMENTATION_PLAN.md) 的执行纪律。

## 0. 定位与边界

- **AA Touch（碰一碰）不是新传输通道，是配对 UX 的替代品**：把"手动输入 PIN 码/扫二维码"换成"物理碰一下"，数据仍然走既有配对协议（`aa4c-proto::Message::PairRequest`/`PairAccept`，见 PROTOCOL.md）——NFC 只负责在两台设备之间物理层面交换一个一次性令牌（充当"我们俩确实靠在一起"的证明，替代人工比对 PIN），真正的配对握手仍然经由现有的设备发现/连接阶梯完成。**这个决定是本设计能收敛到合理工作量的关键**：NFC 单次 HCE 交换的数据量很小，不适合也不需要携带完整配对协议。
- **AA Direct（脱网连接）解决的是"两台设备互相都没有可用网络"的场景**：没有共同 WiFi、没有互联网（因此现有连接阶梯的四档——局域网直连/公网直连/打洞/中继——全部用不了，它们都假设至少有一条能通到公网或局域网的路）。AA Direct 让设备各自开一个临时的点对点连接（WiFi Direct 或蓝牙），充当连接阶梯之外、优先级更高的"第 0 档"：能连上就直接用，不需要以上四档中的任何一档。
- **AA Touch 与 AA Direct 组合使用**：物理碰一下（Touch）触发配对，如果碰完发现双方并不在同一网络下（常见场景：地铁、户外、没有热点），自动退到 AA Direct（WiFi Direct/蓝牙）建立临时连接完成配对与后续传输——这正是 PROJECT_VISION.md 里"第四阶段/第五阶段"顺序安排的原因：Touch 是触发器，Direct 是它常见的落地场景。两者也能独立使用（已配对设备之间直接用 AA Direct 补传输；或者双方本来就在同一 WiFi 下，Touch 配对完直接走局域网直连，不需要 Direct）。
- **关键范围收紧（源自 §1 平台事实调研，本文档最重要的结论）**：
  - **AA Touch 仅限 Android。** 桌面三平台没有面向第三方 App 的可用 NFC API；iOS 对第三方 App 的 HCE 有严格限制（见 §1.1），即便未来做 iOS 也大概率批不下来。
  - **AA Direct 的 WiFi Direct 分支仅限 Android。** 没有跨桌面三平台一致的 WiFi Direct API（Windows 有但 macOS/Linux 没有，做了也是三选一体验，不符合本项目一贯的"桌面三平台一致"原则，见 AGENTS.md/HANDOFF.md 历次教训）。
  - **AA Direct 的蓝牙分支 Android 双向对等，桌面单向（只能扫描/接收，不能广播/被发现）。** 见 §1.3。
  - **蓝牙 Mesh 不进 V0.6，明确后置**——无操作系统级 API，工作量相当于自建一个小型 mesh 协议栈，见 §10。
- **仍然遵守本项目一贯边界**：不做社区/资源平台；AA Touch/Direct 的信任模型复用既有信任分级（完全信任/朋友/临时/陌生，SYNC_DESIGN.md），不新增一套；不碰设备间线路协议版本号以外的东西——`PairRequest`/`PairAccept` 只追加字段，不改已有字段（同 V0.3 `PairServerHint` 的先例）。

## 1. 平台能力实证（不是凭记忆写的——每条都有可查来源）

### 1.1 NFC

- **Android 经典的"碰一碰互传"（Android Beam，NDEF P2P push）已经在 Android 10 弃用、Android 14 彻底移除**——这是本文档最重要的一条事实，直接否定"两台手机一碰直接走 NFC 传数据"这种最朴素的实现路径。（来源：[Android Beam removal, Android 14](https://www.xda-developers.com/android-beam-permanent-removal-android-14/)）
- 现代 Android 要做"碰一碰"效果，必须用 **HCE（Host Card Emulation，`android.nfc.cardemulation.HostApduService`）**：一台设备把自己伪装成一张 NFC 卡片、广播一小段数据，另一台设备用标准 NFC 读卡流程读取。这本质是单向读取（reader 读 emulator），不是双向数据交换——"碰一碰"的双向体验要靠"A 广播 → B 读到 → B 依据读到的信息发起后续网络配对"这个顺序拼出来。
- 官方 `tauri-plugin-nfc`（Tauri 团队维护，[文档](https://v2.tauri.app/plugin/nfc/)）**只支持标签扫描/写入**（`scan()`/`write()`，Android + iOS），**明确不提供 HCE、不提供设备对设备模式**——这个插件的设计目标是"扫商品标签/门禁卡"，不能直接拿来做碰一碰。它对 AA Touch 仍然有用：负责"读"这一侧（B 设备用 `scan()` 读 A 广播出来的 HCE 数据）；但"广播"这一侧（A 设备）必须自己写一个自定义 Android 原生插件（`HostApduService` 子类 + AID 路由注册），是本设计最大的一块新增原生代码。
- iOS Core NFC 从 iOS 17.4 起对第三方 App 开放 HCE，但**仅限欧洲经济区（EEA）用户、需要向 Apple 单独申请专门的 entitlement、且明确限定于"受监管的凭证类交易"场景**（店内支付、封闭式交通卡、车钥匙、门禁、酒店房卡、会员/票务），Apple 官方页面（[HCE for apps in the EEA](https://developer.apple.com/support/hce-transactions-in-apps)）逐条列出的用例里没有"设备间通用数据交换/配对"——即使本项目未来做 iOS，这条路径大概率也批不下来，是产品层面的硬限制，不是工程投入能解决的。
- **结论**：AA Touch 只做 Android；架构上是"读（复用官方插件）+ 广播（自定义 HCE 插件）"两块拼起来，不是单一 API 调用能搞定的。

### 1.2 WiFi Direct

- Android 官方 `WifiP2pManager`（`android.net.wifi.p2p`，[官方指南](https://developer.android.com/develop/connectivity/wifi/wifip2p)）现役、文档里没有弃用提示，Google 官方明确把"设备间文件分享""无基础设施本地同步"列为推荐用例——与 AA Direct 的定位完全吻合，也没有更新的替代 API（"Wi-Fi Aware"是另一个方向，硬件/系统版本要求更高更碎片化，本次不选）。
- 关键约束（会直接影响 UI 文案与实现）：
  - Android 13+（API 33+）需要新权限 `NEARBY_WIFI_DEVICES`（可加 `neverForLocation` 标志，本项目不用这权限做定位）；Android 12 及以下仍然要 `ACCESS_FINE_LOCATION`——这是 Android WiFi 扫描历史遗留的强绑定，不是本项目能绕开的，`discoverPeers()`/`discoverServices()`/`requestPeers()` 还额外要求系统"位置模式"处于开启状态。**必须在 UI 上提前说明"为什么分享文件要开定位权限"，不然用户会误以为 App 想偷偷定位。**
  - 连接模型是协商出一个 Group Owner（GO，充当临时热点）与一个 Client，协商可能失败，要处理 `onFailure` 回调并允许重试；`WIFI_P2P_CONNECTION_CHANGED_ACTION` 等广播在 Android 10+ 变成 non-sticky，要改用 `requestConnectionInfo()` 等主动查询方法，不能只监听广播。
  - 有效范围通常比蓝牙远（官方口径 200 米量级）、吞吐量也更高，适合真正的大文件传输——这也是把 WiFi Direct 定位为"AA Direct 主力传输通道"、蓝牙定位为"握手/小文件兜底"的依据（见 §1.3）。
- **结论**：WiFi Direct 只做 Android；建立好的 P2P 群组本质上就是一个临时局域网（GO 分配到的 IP 段），后续可以直接复用现有的 `aa4c-transfer`（QUIC/TCP）逻辑，不需要为"脱网"重新发明一套传输协议。

### 1.3 蓝牙 / BLE

- Rust 生态成熟的跨平台库是 [`btleplug`](https://github.com/deviceplug/btleplug)（Windows 10+/macOS/Linux/Android/iOS，BSD-3 及 MIT/Apache 双授权，持续维护）——本项目其余部分对"硬件/协议栈用现成成熟库而不是自己撸"是一贯做法（`quinn`/`rcgen`/`rusqlite` 等），蓝牙没有理由例外。**但 `btleplug` 明确只做 central（扫描+连接）角色**——作者原话是"`btleplug` is meant to be *host/central mode only*"，peripheral（广播、被发现）角色需要另找库（作者推荐 `bluster`/`ble-peripheral-rust`），这两个库在跨桌面平台的成熟度都明显不如 `btleplug` 本身。
- Android 原生 API（`BluetoothLeAdvertiser` + `BluetoothGattServer`）同时支持 central 和 peripheral 两个角色，Android 侧可以做到真正对等互相发现。
- **桌面 peripheral（广播）能力薄弱、平台差异大，是这条路线最大的不确定性**。第一阶段收窄范围：**桌面只做 central（扫描/连接 Android 侧广播出来的 AA4C 设备），不做 peripheral（桌面自己不广播、不能被发现）**。拓扑上只有"Android 广播 + 任意设备（含桌面、含其他 Android）扫描连接"这一种方向；桌面之间无法通过蓝牙互相发现——这本来也不是蓝牙近场场景的核心诉求（桌面通常本来就在同一局域网，走既有连接阶梯即可）。
- BLE GATT 吞吐量低（典型几十 KB/s 到大几百 KB/s，取决于 MTU 协商和连接间隔，与 WiFi 差两个数量级），**不适合传大文件**。**结论：蓝牙分支定位为"握手/建连辅助通道 + 小文件兜底"**——BLE 握手成功后，如果双方都是 Android，优先自动升级到 WiFi Direct（吞吐量更高）传实际文件；桌面-Android 场景下蓝牙可能是唯一可用通道，只能接受慢速，UI 要明确提示"经蓝牙传输，速度较慢"。

### 1.4 蓝牙 Mesh——明确不进 V0.6

无论 Android 还是 iOS，都没有操作系统级的蓝牙 Mesh API。Bluetooth SIG 的 Mesh Profile 需要应用自己在 BLE GATT 之上实现完整的配网（Provisioning）、路由、安全协议栈——工作量级与"从零写一个小型 mesh 网络协议"相当，明显超出本里程碑合理范围。**后置到 V0.6 之后独立评估**，见 §10。

## 2. AA Touch（NFC 碰一碰）设计

**流程**（发起方 A 点"碰一碰配对"，被动方 B 用手机碰一下 A）：

1. A 端：调用既有 `PairingManager` 生成一个待确认的配对会话（复用 M4 里程碑的既有机制，见 API_DESIGN.md），但不走"显示 PIN、等对方输入"的既有分支，改走新的"NFC 广播"分支——把这个会话的一次性令牌（不是 PIN 明文，是配对会话内部已有的临时密钥材料的摘要，具体复用现有哪个字段实现期核对 `aa4c-identity::pairing` 源码确定，不新造一套）交给新增的自定义 HCE 插件开始广播。
2. B 端：用户点"碰一碰接收"，App 调用官方 `tauri-plugin-nfc` 的 `scan()` 进入读卡模式；物理一碰，B 读到 A 广播的令牌。
3. B 端：拿到令牌后，走**既有**设备发现（mDNS，同一 WiFi 下）或**既有**远程解析（`resolve_addr`，已配对/知道对方服务器的情况——但这时候 A/B 还没配对，这一分支这里用不上，除非双方都已开启自建服务器公开发现，属于边缘情况，实现期再定）找到 A 的网络地址，然后用令牌替代人工 PIN 输入，完成既有的 `PairRequest`/`PairAccept` 握手。
4. **令牌只在物理接触时通过 NFC 传递一次，不经网络明文传输**——网络层握手仍然走已有的 mTLS + 证书固定协议，令牌只是替代"人工读 PIN 念给对方/对着屏幕输入"这一步，不改变整个配对协议的安全边界。
5. 如果第 3 步在 mDNS 超时时间内找不到对方（说明双方不在同一网络），进入 AA Direct 兜底：B 端提示"没有发现对方，尝试脱网连接"，触发 §3/§4 的 WiFi Direct/蓝牙流程，用同一个令牌完成配对。

**新增原生代码**：一个自定义 Android Tauri 插件（暂定 `aa4c-touch-android` 或直接放进 `apps/desktop/src-tauri/gen/android` 下的自定义插件目录，具体挂接方式实现期查 Tauri 移动插件开发文档确认——本项目至今没有写过正式的 Tauri 插件，只有 `MainActivity.kt` 里对生命周期的简单定制，HCE 需要注册 `HostApduService` 并接收系统 Intent，复杂度明显更高，是 T1 里程碑第一步就要做的技术验证）。

## 3. AA Direct（WiFi Direct，Android）设计

- 新增 `aa4c-transfer::WifiDirectDialer` trait，形状仿照既有 `PunchDialer`/`RelayDialer`（见 `crates/aa4c-transfer/src/lib.rs`，`OnceLock<Arc<dyn Trait>>` 注入模式）：桌面壳层的等价实现直接返回"不可用"（因为只有 Android 支持），Android 壳层注入真正调用 `WifiP2pManager` 的实现。
- 触发时机：连接阶梯现有四档（局域网直连/公网直连/打洞/中继）全部失败，且本机是 Android，才尝试 WiFi Direct——定位成阶梯之外的兜底，不改变阶梯本身的既有顺序和语义（`ConnectionVia` 枚举新增 `WifiDirect` 变体，同 C4/C5 加 `Punch`/`Relay` 变体的先例）。
- 群组协商出的 IP 是一个临时局域网地址，拿到之后直接复用现有 `aa4c-transfer` 的 QUIC/TCP 连接与传输逻辑——不新造传输协议。

## 4. AA Direct（蓝牙）设计

- 新增 `aa4c-transfer::BleDialer` trait，同样仿照既有 dialer 注入模式。
- 桌面壳层实现：基于 `btleplug`，只做 central（扫描 AA4C 设备的 BLE 广播、发起 GATT 连接）。
- Android 壳层实现：原生 `BluetoothLeAdvertiser`/`BluetoothGattServer` + `BluetoothLeScanner`，central/peripheral 双角色都做。
- 通过 BLE GATT 特征值交换的是**小数据**（配对令牌、控制消息），不是文件内容本身——真正的文件传输，Android-Android 场景下握手成功后立即尝试升级到 WiFi Direct；桌面参与的场景下，评估直接用 GATT 分片传输小文件是否够用，大文件场景明确提示用户"先接入同一网络"，不打算在 V0.6 让 BLE 自己扛住大文件传输。

## 5. 数据模型 / 设置项

- `Settings` 新增（均默认关闭，配对/直连都属于会主动暴露设备的能力，不该悄悄打开）：`touch_enabled: bool`（AA Touch 总开关）、`direct_enabled: bool`（AA Direct 总开关，控制 WiFi Direct + 蓝牙两个分支）。
- `aa4c_types::CoreEvent::ConnectionVia`（现有枚举）新增 `WifiDirect`、`Ble` 两个变体，同 C4/C5 加变体的既有先例（只追加，不改已有）。
- 不新增数据库表——AA Touch/Direct 都是连接建立层面的能力，不产生需要持久化的新业务实体（配对结果仍然写进既有 `devices` 表，传输记录仍然写进既有 `transfer_tasks` 表）。

## 6. 安全与隐私

- HCE 广播/BLE 广播都只在用户主动点击"碰一碰"/"脱网连接"按钮后的一个有限时间窗口内开启（比如 60 秒超时自动停止），不是常驻广播——避免设备在口袋里的时候被陌生人扫描/触发。
- NFC/BLE 传递的令牌是一次性的、与具体这次配对会话绑定，用过即失效，不可重放。
- WiFi Direct/蓝牙建立的连接完成配对后，安全边界与既有局域网直连完全一致（同一套 mTLS 证书固定协议），AA Touch/Direct 不引入新的信任判断逻辑，复用既有信任分级。
- 蓝牙广播的设备标识不能是长期不变的真实设备 ID（会变成可追踪的稳定信标）——用配对会话临时生成的随机标识，具体方案实现期在 `aa4c-identity` 里核对现有临时会话 ID 生成方式是否可以直接复用。

## 7. UI

- 设置页新增「碰一碰 / 脱网连接」区块：两个开关（AA Touch、AA Direct），文案说明"仅 Android 支持""需要开启位置权限（Android 系统要求，本 App 不用它定位）"。
- 配对流程页新增"碰一碰配对"入口（旁边保留既有 PIN 配对，不是替换）。
- 传输/配对进行中如果经由 WiFi Direct/蓝牙，UI 上要显式标出（同 C4 连接质量徽标"直连/中继（较慢）"的既有先例，这里加"脱网直连"/"蓝牙（较慢）"）。
- 文案不出现"HCE"/"GATT"/"P2P"这类术语（同 AGENTS.md 既有的文案纪律），用"碰一碰"/"脱网连接"/"附近直连"这类用户语言。

## 8. 里程碑与验收（详细步骤见 V0.6_IMPLEMENTATION_PLAN.md）

| 里程碑 | 内容 | 交付判定 |
|--------|------|------|
| T1 | AA Touch（Android HCE 自定义插件 + 复用官方 `tauri-plugin-nfc` 读取 + 配对流程接线） | **需要用户用两台真实 Android 设备碰一碰验证**，本环境无法自测 |
| T2 | AA Direct WiFi Direct（Android，`WifiDirectDialer` + 接入连接阶梯兜底） | **需要用户用两台真实 Android 设备在断网环境下验证**，本环境无法自测 |
| T3 | AA Direct 蓝牙（`btleplug` 桌面 central + Android 原生双角色 + `BleDialer`） | **需要用户用真实蓝牙硬件验证**（至少一台 Android），本环境无法自测 |
| T4 | 收尾：全量验证 + 文档 + `v0.6.0-preview` 发布 | 三平台 + Android APK 发布产物齐全 |

## 9. 已确认决策表

| 决策 | 内容 | 依据 |
|------|------|------|
| AA Touch 平台范围 | 仅 Android | §1.1 |
| AA Direct WiFi Direct 平台范围 | 仅 Android | §1.2 |
| AA Direct 蓝牙平台范围 | Android 双角色对等，桌面仅 central | §1.3 |
| 蓝牙 Mesh | 不进 V0.6，后置 | §1.4/§10 |
| NFC 数据语义 | 只传一次性配对令牌，不传文件/协议全量数据 | §2 |
| 传输协议 | WiFi Direct/蓝牙建好连接后复用既有 QUIC/TCP 传输，不新造协议 | §3/§4 |
| 大文件策略 | 蓝牙不扛大文件，Android-Android 场景握手后升级 WiFi Direct | §1.3/§4 |
| 广播时机 | 仅用户主动触发的有限时间窗口，不常驻 | §6 |
| iOS | 明确排除，同项目至今未落地 iOS 的既有状态一致 | §0 |

## 10. 实现期必须补的实证 + 明确后置项

**实现期开工前必须做（本环境做不了，需要用户配合，同 V0.5 AI2.0 的"前置实证"纪律，但这次实证责任在用户）**：

1. 用真实 Android 设备确认 `HostApduService` 的最小可行 demo：广播一段自定义数据，另一台设备（或同一台开发机连的另一台测试机）用官方 `tauri-plugin-nfc` 的 `scan()` 真的能读到——在写任何 AA4C 业务逻辑之前，先跑通这个最小闭环，同 AI2.0"先证明技术路径可行，再动业务代码"的既有纪律。
2. 确认 Tauri 移动端自定义插件的准确挂接方式（本项目至今没写过正式 Tauri 插件，只有 `MainActivity.kt` 生命周期定制这一种更简单的先例）——查 Tauri 官方移动插件开发文档，不要凭 `tauri-plugin-nfc` 源码倒推瞎猜。
3. `WifiP2pManager` 最小可行 demo：两台 Android 设备真实建立 P2P 群组、互传一个文件，确认 Group Owner 协商成功率、实际吞吐量、真实需要哪些权限弹窗。
4. `btleplug` 桌面 central 模式最小可行 demo：真实扫描到一台 Android 设备（先用系统自带蓝牙广播工具或简单测试 App 模拟广播源）、建立 GATT 连接、读写一个特征值。
5. Android `BluetoothLeAdvertiser`/`BluetoothGattServer` 最小可行 demo：真实广播 + 被另一台设备（或 `btleplug` 跑在桌面上）连接读取。

**明确后置、不进 V0.6**：蓝牙 Mesh（§1.4）；iOS 全平台（同项目现状）；NFC 标签写入场景（比如"贴一张 NFC 贴纸在门口自动同步"，是 `tauri-plugin-nfc` 已有能力但不是本里程碑目标）；WiFi Aware（比 WiFi Direct 更新但硬件/系统版本要求更碎片化，当前不选）。
