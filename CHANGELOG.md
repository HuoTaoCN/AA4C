# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式与[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- V0.3 同步里程碑 C5：NAT 打洞（连接阶梯第 3 档，排在中继之前）——**V0.3「AA Connect」六个里程碑至此全部完成**。`aa4c-server` 新增一个轻量 QUIC 反射端点（`aa4c_server::reflect`，与 TCP 信令同端口号，独立 ALPN `aa4c-reflect`）：设备用自己**真正用于 P2P 的那个 QUIC 端点**连一次，服务器把 `Connection::remote_address()` 经一条 uni 流回给它——自建版 STUN binding response，不引入公共 STUN 依赖，且保证反射地址与后续打洞用的是同一个本地端口/NAT 映射。`ServerMessage` 追加（只追加变体）：`Signal{target,candidates}`（C→S，在发起方**自己的常驻连接**上发出）/`IncomingSignal{from,candidates}`（S→C，推给目标的常驻连接，复用 C3 已有的 `pushable` 推送表，不需要新基础设施）。`aa4c-core::server_link` 新增 `SignalChannel`（outbox + 按 device_id 键控的 pending oneshot 表）：外部调用方把候选请求"塞进"常驻连接的写侧并等待对端回信；常驻连接收到 `IncomingSignal` 时先判断是不是自己正在等的回信——**是**就转交给等待者，**不是**才需要反向探测候选 + 打几个尽力而为的探测包 + 回信，这个区分是关键：不做的话两边会对同一次交换无休止地互相"回信"（实现时真实踩到的死循环）。`aa4c-transfer` 新增 `PunchDialer` trait（`dial()` 插入连接阶梯第 3 档：直连失败后先试打洞，候选逐个 `quic::connect`，第一个成功即为 `ConnectionVia::Punch`，全部失败或候选交换本身失败则落中继）与 `TransferService::reflexive_addr`/`punch_probe` 公共方法。`aa4c_types::ConnectionVia` 追加 `Punch`；前端 UI 上 `Punch` 并入「直连」显示，不单独暴露成第三个词（打洞只是找到对方的手段，连上之后就是货真价实的直连）。
  - 过程中发现并修复两个真实问题（不是本里程碑范围内的新代码，但被本里程碑第一次踩到）：(1) **QUIC 连接过早释放**——`quic::QuicDuplex` 此前只是 `tokio::io::Join<RecvStream, SendStream>` 的类型别名，不持有 `quinn::Connection` 句柄；入站分流里"转交给 Core 钩子后立即返回"的分支（`IndexRequest`，钩子内部自己 `tokio::spawn`、不等它跑完）会导致本地 `Connection` 变量随分发函数返回而丢弃，钩子的后台任务还没来得及读写就先撞见连接被拆（`Offer`/`FetchRequest` 因为全程 `.await` 到底不受影响）——这是从 C1 引入 QUIC 起就潜伏的 bug，直到 C5 打洞路径才第一次把"纯 QUIC + IndexRequest"这个组合跑起来，暴露出来。修复：`QuicDuplex` 改为持有 `Connection` 的结构体，手动转发 `AsyncRead`/`AsyncWrite`；同时 `serve_index`（`aa4c-core::dispatch`）写完最后一批消息后不再直接返回丢连接，而是显式半关闭写侧再读到对端也关闭它那侧为止（`finish_write_side`），因为"写成功"只代表进了本地发送缓冲区，不代表已经送达。(2) **回环环境打洞会截胡中继测试**——C3 的 `forced_relay_path_completes_a_transfer` 在打洞加入后，因为回环没有真实 NAT、打洞稳定成功且排在中继之前，其实已经在悄悄测打洞而不是中继；新增 `TransferConfig::disable_punch` 测试/联调专用开关（同 `prefer_quic` 的先例）让该测试能确定性地绕过打洞，并补上 `ConnectionVia` 断言防止同类回归再次静默发生。新增 Core e2e：`forced_punch_path_completes_a_transfer`（强制走打洞完成一次真实文件传输，断言 `ConnectionVia::Punch`）；`aa4c-server` 新增反射端点单测 + `Signal`/`IncomingSignal` 推送状态机单测（含目标未注册时静默丢弃）。

- V0.3 同步里程碑 C4：远程同步/发送接入完整连接阶梯 + 连接质量 UI——**里程碑 C1–C4 至此全部完成，远程能力端到端可用**。`sync_exchange`（跨设备索引交换）此前只认 mDNS 在线快照，远程（跨网络）完全信任设备永远同步不到，现改为遍历全部完全信任配对设备，逐个走共享的地址解析阶梯（新增 `aa4c-core::orchestrate::resolve_addr`，`resolve_peer`/`sync_exchange` 共用同一套「mDNS → 落库最后地址 → 服务器 Lookup」，不再各自维护一份）；触发策略上 `DeviceFound` 仍即时触发（局域网设备快），新增 30s 周期定时器兜底远程设备（`DeviceFound` 对它们永远不会触发）。`aa4c-transfer::TransferService::fetch_index`/`fetch_file` 的 `addr` 参数改为 `Option<SocketAddr>`，解析不出地址或直连失败时同 `send()` 落中继兜底（C3 已给 `send()` 接好，这次补齐索引交换/按需拉取两条路径）。`Core::fetch_file` 不再局限于 mDNS 在线持有者，改为遍历全部持有者（mDNS 在线优先排前）逐个走 `resolve_peer`。在线判定扩展：统一视图的 🟢/🟡/🔴 归并从「仅 mDNS 在线」扩展为「mDNS 在线 **或** 最近一次远程索引同步仍在新鲜窗口内」（90s，约 3 倍周期定时器间隔）——用"最近同步成功过"代替另起一次实时探测，注意这不是绝对保真，拉取失败前端仍给温和提示+可重试。连接质量：`aa4c_types` 新增 `ConnectionVia`（`Direct`/`Relay`）与 `CoreEvent::TransferConnected{task_id, via}`，`aa4c-transfer::dial()` 返回值带上实际走的档位，`send.rs`/`fetch.rs` 在 `dial()` 成功后广播一次（只有发起方——发送/拉取——收得到，只存当次会话内存不落库）。前端：`Settings` 类型补齐 `serverUrl`/`enableRemote`，设置页新增「远程连接」区块（服务器地址输入 + 开关，地址为空时开关禁用并给出提示）；`useTransferStore` 新增 `onConnected`（懒创建占位任务，兼容拉取路径没有预先登记骨架的情况），`TransferCard.vue` 按 `via` 显示「直连」/「中继（较慢）」徽标。新增测试：`aa4c_types` 的 `TransferConnected` JSON 形状测试；Core 全链路 e2e `remote_index_exchange_reaches_peer_via_relay`（复用 C3 的「逼连接阶梯只剩中继」手法，验证完全信任设备的共享索引能纯靠中继同步过来）。
- V0.3 同步里程碑 C3：`aa4c-server` 中继面——连接阶梯「局域网直连 → 公网直连 → 中继」贯通，**远程可用自此成立**。`ServerMessage` 追加（只追加变体，不改既有）：`RelayRequest{target}`/`RelayGrant{session_token,ttl_secs}`（C→S 申请一次性短 TTL token，8s，`aa4c_server::RELAY_TOKEN_TTL`）、`IncomingRelay{session_token,from}`（S→C 推给被叫方常驻连接）、`RelayOpen{session_token}`/`RelayOpenAck{ok}`（C→S 新连接，凭 token 撮合）。服务器侧：`RelaySlot`（`oneshot` 把先到者的连接交给后到者所在的任务）撮合两条 `RelayOpen` 连接，两侧到齐才回 `RelayOpenAck{ok:true}`，随后连接**转入裸字节透明转发**（`tokio::io::copy_bidirectional`）——对设计初稿 `RelayData`/`RelayClose` 的一处收敛：省去逐包重新编解码帧头，效果等价（服务器仍只盲转发字节、不解密），token 一次性，被首次 `RelayOpen` 触碰即从登记表移除（无论撮合成败），定期清扫从未被触碰的过期条目。`RelayRequest` 不检查允许名单（服务器不理解"配对"语义）——真正的安全边界在被叫方自己：中继裸管道撮合后跑的仍是设备间 mTLS + `dispatch_shared` 的 `trusted` 检查，和直连路径完全同构（详见 PROTOCOL.md §15）。客户端侧（`aa4c-core::server_link`）：`enable_remote=true` 时维持**一条常驻连接**周期续约 `Register` 并 `select!` 监听 `IncomingRelay` 推送（这是被叫方能收到中继会话通知的前提，CONNECT_DESIGN.md §3.4）；设置变更/解除配对用新增的 `tokio::sync::Notify`（`spawn_register_loop` 返回，`Core` 持有）唤醒这条连接**立刻**重新注册，不再另开一次性连接——早期实现让一次性连接也发 `Register`，会与常驻连接抢服务器侧的推送登记槽位，一次性连接断开时的清理会把常驻连接刚登记好的活通道顶掉，直到下一轮周期续约（最长 TTL/3）才恢复，这段窗口内的中继推送会悄悄丢失（实测踩到的真实竞态，不是假设）；现在只有一条连接会调用 `Register`，从根上不存在竞争。`aa4c-transfer` 新增 `RelayDialer` 注入 trait（`dial()` 返回中继裸管道，未叠加设备间 TLS——`TransferService::dial` 收到后像对待新拨的 TCP 连接一样在其上再做一次 `TlsConnector::connect`，与直连完全对称）与 `TransferService::accept_external`（把中继撮合好的裸管道接入与 TCP/QUIC 入站完全相同的 TLS-accept + 分流管线，新增 `recv::run_incoming_external`）；`dial()` 签名从 `addr: SocketAddr` 改为 `addr: Option<SocketAddr>`——前三档都没解析出地址时不再提前报错，直接尝试中继；直连失败时同样落到中继兜底。`aa4c-core::Core::start` 装配阶段注入 `RelayDialerImpl`（申请 `RelayRequest`/`RelayOpen`，`aa4c-transfer` 不感知服务器协议细节）。测试：`aa4c-server` 新增 4 个确定性单测（撮合裸字节双向可达、未知 token 拒绝、过期 token 拒绝、token 一次性不可复用）；新增 Core 全链路 e2e `forced_relay_path_completes_a_transfer`（关掉 A 自己的 mDNS + 把 B 的落库地址钉成确定关闭的端口逼连接阶梯只剩中继一档，真实走完一次文件传输并校验内容）。
- V0.3 同步里程碑 C2：`aa4c-server` 自建信令面 + 客户端接入——第一次让不在同一局域网的已配对设备有了「找到对方」的手段。新 crate `crates/aa4c-server`（lib + bin）：身份复用 `aa4c-identity`（独立数据目录），服务器地址格式 `aa4c://host:port#指纹前16位`（`aa4c_types::ServerAddr` 解析）；鉴权复用 mTLS（服务器接受任意合法 Ed25519 客户端证书，从证书读出查询/注册方身份），**未实现设计初稿的 `Challenge`/`ChallengeReply`**——TLS 握手本身已密码学证明身份，绑定在整条连接上，语义等价、少一次往返、不引入独立签名依赖（详细理由见 PROTOCOL.md §11）。`aa4c-proto` 新增独立的 `ServerMessage` 协议族（`aa4c_proto::server`，独立 `server_proto` 版本，同守「只追加变体」）：`SrvHello`/`SrvHelloAck`、`Register{endpoints,proto,allow_list}`/`RegisterAck{ttl_secs}`、`Lookup{device_id}`/`LookupReply{endpoints}`；帧层复用（`encode_frame`/`read_message`/`write_message` 泛型化，两套协议共用同一套帧实现）。服务器注册表全内存态（无持久化，进程重启即清空，客户端周期续约自愈）：`Register` 覆盖式整体替换登记，**吊销自然发生**——不需要任何显式吊销协议，下一次注册的允许名单里没有对方就查不到了；服务器额外把连接观测到的源地址并入返回端点（免 STUN 的反射地址）。TTL = 60s（`aa4c_server::REGISTER_TTL`），客户端约每 TTL/3 续约（`aa4c-core` 新模块 `server_link.rs` 的后台循环），设置变更 / 解除配对会立即触发一次续约。`Settings` 新增 `server_url`/`enable_remote`（默认 `false`，隐私优先）；`resolve_peer` 增加第三档兜底：mDNS → 落库最后地址 → 向**自己配置的服务器** Lookup（跨服务器好友寻址需要配对协议交换 `devices.server_hint`，这个字段已建表但线路层交换留待后续——`PairRequest`/`PairAccept` 是既有 bincode 结构体，追加字段会破坏 v1/v2 解码，是本里程碑有意缩小的范围）。交付物含 `crates/aa4c-server/Dockerfile`、`scripts/dev-server.sh`、CI release 新增 Linux 二进制产物。新增确定性单测（`aa4c-server` 自身 4 个 + `aa4c-core::server_link` 3 个，均不经 mDNS/Core，直接驱动协议）覆盖注册/查询/允许名单拒绝/吊销；新增 e2e 测试验证 Core 全链路能靠服务器兜底连上从未直连过地址的配对设备，以及解除配对后 B 端本地信任判定正确拒绝后续传输（这台开发机上真实 mDNS 组播确实会在数百毫秒内互相发现，测试改为断言异步 `TransferFailed` 事件而非 `send_files` 的同步返回值，理由见测试内文档注释）。
- V0.3 同步里程碑 C1：广域网 QUIC 会话层 + 断点续传（首个突破局域网的能力）。`aa4c-transfer` 新增 `quic.rs`：证书固定复用现有 rustls 配置（与 TCP 同一套 mTLS 信任模型），ALPN 固定 `aa4c`，UDP 端口与 TCP 同号（`start_listener` best-effort 绑定，失败只警告回落纯 TCP）；keep-alive（2s）+ 空闲超时（8s）——应用层等待用户确认可长达 60s，心跳持续续命，只有心跳也送不出去的真断连才会被发现。出站是否走 QUIC 由 `TransferConfig.prefer_quic` 控制（当前仅测试/联调开关，默认 `false` 零行为变化；「按可达性自动选择」收口在里程碑 C4）。`aa4c-proto` 新增 `Message::ResumeReport{task_id, progress}`（向后兼容追加变体）：双方协商 `proto ≥ 3` 时接收方**确定性**回报每个文件的可信续传起点（`.aa4c-part` 按 4 MiB 边界截断，只信任完整写入过的整块，不做逐块签名比对——最终 `FileDone` 仍会校验整文件哈希兜底），发送方从该偏移重新流式读源文件前缀喂哈希后续传，不重发已确认部分。同步调整清理策略：只有「明确取消」（本地用户取消 / 对端主动 `Cancel`）才清理 `.aa4c-part`，网络掉线等意外中断保留 part 文件——这正是续传的前提（连带修了一个真实的既有小缺口：发送方 `send_file` 内部检测到取消时此前不会通知对端，导致对端只能空等超时而非按 PROTOCOL §7 规则 3 立即清理）。`PROTO_VERSION` 升到 3；`aa4c-core`/`aa4c-transfer`/`aa4c-proto` 新增 `IncomingIndexDispatch`/入站分流泛化（`SharedStream`），使索引交换与按需拉取在 TCP 与 QUIC 两种承载层上复用同一套逻辑（配对暂仍限局域网 TCP，V0.3 未做远程配对）。新增 workspace 依赖 quinn（`rustls-ring` 特性，与现有 rustls/ring 版本树完全对齐、零重复加密后端）；`rust-version` 相应升到 1.85。新增 e2e 测试 `quic_roundtrip_transfer`（真实 QUIC 端到端传输）与 `quic_resume_after_disconnect`（UDP 黑洞代理模拟真实网络分区，验证断连后重新发起能正确续传且最终内容/哈希正确）。
- V0.3 设计文档（定稿 v2，经评审修订）：新增 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)（AA Connect —— 突破局域网）与 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)（里程碑 C1–C6 实现计划，C1 细化到步骤级）。核心决策：基础设施**仅自建**且**单进程 `aa4c-server`**（信令+中继合一，token 进程内校验）；服务器身份与设备同构（密钥对 + 自签证书，**指纹写进地址** `aa4c://host:port#fp`，零 CA / 零域名）；查询授权用**允许名单（注册时上传）+ 挑战应答**（初稿互签 proof 因吊销漏洞弃用，解除配对即自然吊销、不依赖设备时钟）；寻址「查谁去谁的 home server」（`devices.server_hint` 正式字段）；信令协议复用帧层 bincode 长连接（不引入 HTTP/WS）；设置收敛单 `server_url` + `enable_remote` **默认关**；QUIC 首版**单流等价迁移**（复用既有泛型收发循环），断点续传用追加变体 `ResumeReport`（不改既有 `Offer`）；分享仅限已配对好友 + 已索引内容（复用 `resolve_shared` 边界）+ 默认只读；里程碑顺序**中继先于打洞**（打洞降级为提速优化，远程可用在 C3 成立）。同步更新 DATABASE_SCHEMA（§4c `shares`/`share_access`/`devices.server_hint`/settings 键）、PROTOCOL（Part B §10/11/13/15 重写）、ROADMAP、HANDOFF。仅设计，未实现。

## [0.2.0-preview.2] - 2026-07-03

> **预览版（第二版）**：V0.2 **跨设备同步从设计稿变为可用**——共享文件夹 + Inbox、跨设备统一文件视图（🟢 本地有 / 🟡 可下载 / 🔴 设备离线）、点黄色即从在线设备拉取并转绿、同名不同内容「多版本」并列可分别取回；实时文件监听秒级更新。已过真机双实例 GUI 走查。
>
> ⚠️ **与 `v0.2.0-preview` 跨设备同步不互通**：线路协议升到 `proto = 2`，索引/拉取按版本门槛跳过 `proto=1` 的旧预览版；基础的发现 / 配对 / AA 直传仍可用。参与同步的设备请都升级到本版本起的构建。（与 v0.1.x 仍因 `DeviceInfo.trust_level` 无法配对。）

### Changed

- 协议版本升到 `proto = 2`，并给 V0.2 新增的**索引交换 / 按需拉取**加发起方版本门槛：握手 `Hello.proto` 取双方最小值，只有协商结果 `≥ 2`（`aa4c-types::SYNC_PROTO_VERSION`）才发送索引/拉取消息；遇到 v1 对端直接不发（优雅降级为纯 v1 传输），不再依赖对端 bincode 解码失败断开来兜底。落点：`TransferService::fetch_index` 与 `fetch.rs` 握手后判断；mDNS TXT 的 `proto` 字段也随之广播为 `2`。新增 proto 测试 `client_hello_negotiates_down_to_v1_peer`。
  > ⚠️ 与 `v0.2.0-preview`（广播 `proto=1`）的**跨设备同步不再互通**（索引/拉取会被版本门槛跳过）；基础的发现 / 配对 / AA 直传仍可用。同步联调请让两端都升级到本版本起的构建。趁 preview 仍是预发布窗口一次性把线路版本对齐，避免正式版再背历史包袱。
- 桌面端联调钩子：环境变量 `AA4C_DATA_DIR` 覆盖数据目录（含接收目录 `Inbox`）、`AA4C_DEVICE_NAME` 指定首启设备名，使同一台机器能跑多个互相隔离的实例做双机真机联调（真 mDNS / 真 TLS / 真 GUI）。仅开发用途，默认不影响正常启动。

### Added

- 同步实时文件监听：接入 `notify`（`notify-debouncer-mini`，2s 去抖）监听各共享范围目录，增删改秒级触发重扫（原来最长要等 300s 定时扫描或一次传输完成）。监听目录随共享范围增删自动对齐；定时 300s + 传输完成仍作兜底（监听盲区/不可用时静默退化）。完成 SYNC_DESIGN §3.2 设计的「文件监听 + 定时兜底」终态。新增测试 `watcher_rescans_on_filesystem_change`。
- 按需拉取落点镜像回原范围：拉取黄色文件时按限定路径顶层分组段匹配本机共享范围，命中则**落回该范围原目录结构**（文件夹来源文件回到原文件夹、原黄条目直接转绿），未命中才回落 Inbox（`Core::fetch_file` 计算落点）。此前一律落 Inbox，文件夹来源文件不会与原黄条目并条。
- V0.2 同步里程碑 5：冲突标记与人工解决——**V0.2 同步五个里程碑至此全部完成**。同一限定基准路径出现多个不同 hash 的版本时，统一视图并列展示、以序号区分（`报告.pdf` / `报告 (2).pdf`），各自独立着色（🟢/🟡/🔴）、可分别拉取（拉取按 `basePath` + `hash` 定位具体版本）。`unified::merge` 改为按 (rel_path, hash) 拆分版本；`UnifiedFile` 增加 `basePath` / `conflict` 字段。当前冲突整体落 `sync_conflicts`（迁移 `005_conflicts.sql`，user_version=5；每版本一行，`(rel_path,hash)` 主键，支持多方冲突；单事务 diff 保留 `created_at`）。**绝不自动覆盖**：拉取想要的版本落盘时 `.aa4c-part` 自动加序号与本地副本共存、各自转绿，冲突随下次刷新消解。新增 Tauri 命令 `list_conflicts`，`fetch_file` 增加 `hash` 参数；「同步」页冲突文件显示「多版本」标记。
- V0.2 同步里程碑 4：按需拉取——点统一视图里黄色「可下载」文件即可从在线设备取回到本机，完成后自动转绿。`aa4c-proto` 新增 `FetchRequest{rel_path}` 消息（向后兼容追加）；拉取方连持有方、发 `FetchRequest`，持有方校验**完全信任** + 路径落在共享范围内后**反转角色**，在同一连接上复用既有发送流（`Offer`→分块→`FileDone`/`FileAck`→`TaskDone`）回推内容，拉取方自动接受，不新增数据通路。安全边界：只解析本机**已索引（已对外广播）**的条目，绝不按对端任意路径读盘、天然挡掉 `..` 穿越；非 full 一律 `Cancel`。落盘剥掉顶层来源分组段后进 Inbox——「收到的」来源文件回到同一限定路径并入同一条目转绿（文件夹来源文件先落 Inbox，按范围镜像回原结构留作后续）。传输层重构出可复用的 `receive_files` / `serve_fetch`（推送与拉取共用一套分块/BLAKE3 路径）。新增 Tauri 命令 `fetch_file`；「同步」页点黄色文件即拉取 + toast 反馈。新增端到端测试 `on_demand_fetch_pulls_file_and_gates_on_full_trust`（真实 TLS 验证「朋友拒绝拉取、我的设备拉取成功并落盘」）。
- V0.2 同步里程碑 3：跨设备索引摘要交换 + `remote_index` + 统一文件视图（绿/黄/红，只读）。`aa4c-proto` 新增 `IndexRequest` / `IndexEntries` 消息（向后兼容追加变体，旧版优雅降级）；设备上线即与**在线的完全信任设备**交换索引摘要（只传元数据 rel_path/size/hash，不传内容），整体落 `remote_index`（迁移 `004_remote_index.sql`，user_version=4）。完全信任边界在持有方把关：非 full 对端一律回空批次、不泄露任何文件名；完全信任降级为朋友时立即清空该设备的远端索引缓存。统一视图 `unified::merge` 把本机索引 ⊕ 远端索引按限定路径（顶层段=「收到的」/共享文件夹名）归并，按「本机有→🟢 / 在线设备有→🟡 / 仅离线设备有→🔴」着色并标注持有设备。新增 Tauri 命令 `list_unified_files` / `refresh_remote_index`；「同步」页接统一视图（绿/黄/红 + 持有设备 + 筛选），新增「刷新设备」按钮。拉取触发=启动初拉 + 设备上线触发 + 手动刷新（未接 `notify`/`IndexSummary` 优化，见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §11）。跨设备**按需拉取**内容仍属里程碑 4。
- V0.2 同步里程碑 2：共享范围 + 本地文件索引扫描 + Inbox 落点，「同步」页接上真实文件（不再是示例数据）。`sync_scopes` / `sync_file_index` 落库（迁移 `003_sync.sql`）；`aa4c-core` 新增扫描器（mtime+size 未变复用旧 BLAKE3、变化则重算，跳过隐藏文件与 `.aa4c-part` 临时文件），启动时扫一次、之后每 300s 定时全量重扫，任意一次传输完成也会追加一次扫描；`save_dir` 变更时 Inbox 范围自动跟随并清空旧路径下的索引。新增 Tauri 命令 `list_sync_scopes` / `add_sync_scope` / `remove_sync_scope` / `list_sync_files` / `rescan_sync` 与 `sync_index_updated` 事件。「同步」页支持添加/移除同步文件夹、手动重新扫描；统一文件视图当前恒为绿（本地有）——跨设备黄/红状态待里程碑 3 的索引摘要交换落地。文件系统实时监听（`notify`）暂未接入，先用定时扫描兜底（见 [SYNC_DESIGN.md](SYNC_DESIGN.md) §11）。

## [0.2.0-preview] - 2026-06-29

> **预览版**：品牌重塑 + 新能力导航 UI + 信任分级（第一步）已落地，跨设备文件索引/同步仍是设计稿（见 [SYNC_DESIGN.md](SYNC_DESIGN.md)），尚未实现。
>
> ⚠️ **配对协议不兼容**：`DeviceInfo` 新增 `trust_level` 字段改变了配对阶段交换的数据结构，本版本与 v0.1.x 之间**无法互相配对**——升级请确保参与配对的设备都更新到本版本（同版本之间不受影响）。

### Changed

- 产品定位升级为「**AA连接（AA4C）—— 开源跨平台设备连接平台**」（设备 → 连接 → 能力，连接优先）：明确不做社区/资源平台/中心化云盘；Slogan 改为"连接你的所有设备"；新增"连接优先"五阶段路线（AA Nearby → Sync → Connect → Touch → Direct）与 D2D 未来方向；移动端确认沿用 Tauri 2（iOS/iPad/平板由同一构建+响应式覆盖，Flutter 仅远期备选）。同步更新 README / PROJECT_VISION / ROADMAP / ARCHITECTURE / AGENTS / CONTRIBUTING / UI_DESIGN_SPEC / CODEX_MASTER_PROMPT。

### Added

- V0.2 信任分级（数据模型，第一步落地）：`TrustLevel`（full / friend）类型；`devices.trust_level` 列 + 迁移 `002_trust.sql`（旧已配对设备回填 friend）；`Store::set_trust_level`、`Core::set_trust_level` 与 Tauri `set_trust_level` 命令；配对成功默认 friend（重配对保留已有 full），`DeviceInfo.trustLevel` 贯穿到前端；配对成功即弹「这是你自己的设备吗？」与设置页「我的设备 ⇄ 朋友」均已接真实后端。索引/同步（full 设备间）属后续阶段。
- V0.2 设计文档：新增 [SYNC_DESIGN.md](SYNC_DESIGN.md) —— 设备信任分级（完全信任/朋友/临时/陌生）、跨设备文件索引、文件状态可视化（🟢 本地有 / 🟡 可下载 / 🔴 设备离线）、元数据优先+按需获取、Inbox「收到的」纳入索引；同步更新 PROJECT_VISION（权限分级 + 同步）、DATABASE_SCHEMA（V0.2 表：`devices.trust_level` / `sync_scopes` / `sync_file_index` / `remote_index` / `sync_conflicts`）、UI_DESIGN_SPEC（同步页统一文件视图 + 设置页信任层级）、ROADMAP。仅设计，未实现。
- 前端能力架构：导航围绕五大能力（传输/同步/分享/下载/归档）重构，首页能力卡片 + 建设中页 + PC 侧栏/移动底栏两套外壳；界面品牌改为「AA连接」。
- UI 设计预览（示例数据，后端 V0.2 接）：同步页跨设备文件**目录树**（可展开）+ 文字状态标签（本地有 / 可下载 / 设备离线）+ 图例 + 筛选 + Inbox 分组；文件被多台在线设备持有时提示「同时取回（更快）」。
- 信任分级入口前移：配对成功即弹「这是你自己的设备吗？（是，我的设备 / 不是，朋友）」；设置页保留「我的设备 ⇄ 朋友」分段切换。

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

[Unreleased]: https://github.com/HuoTaoCN/AA4C/compare/v0.2.0-preview.2...HEAD
[0.2.0-preview.2]: https://github.com/HuoTaoCN/AA4C/compare/v0.2.0-preview...v0.2.0-preview.2
[0.2.0-preview]: https://github.com/HuoTaoCN/AA4C/releases/tag/v0.2.0-preview
[0.1.1]: https://github.com/HuoTaoCN/AA4C/releases/tag/v0.1.1
[0.1.0]: https://github.com/HuoTaoCN/AA4C/releases/tag/v0.1.0
