# AA4C 远程连接与分享设计（V0.3）

> 状态：**设计定稿（v2，经评审修订）**，对应 [ROADMAP.md](ROADMAP.md) V0.3（AA Connect）。本文档是落地依据，不含实现代码；实现拆解见 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)。
> 关联：线路层权威见 [PROTOCOL.md](PROTOCOL.md) Part B（proto ≥ 3 广域网）；信任分级见 [PROJECT_VISION.md](PROJECT_VISION.md) §十；表结构见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)；同步复用见 [SYNC_DESIGN.md](SYNC_DESIGN.md)；界面见 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md)。
> 评审修订（相对初稿）：服务器身份改为**密钥对 + 地址内指纹固定**；配对证明（proof）方案**删除**，改为**注册时上传允许名单 + 挑战应答**；Rendezvous 与 Relay **合并为单进程 `aa4c-server`**；信令协议**复用帧层 bincode 长连接**（不用 HTTP/WS）；设置收敛为**单 `server_url`**（默认关闭远程）；分享**仅限已索引内容**；里程碑顺序 **Relay 提前到打洞之前**。

## 1. 背景与目标

V0.1 只在同一局域网内工作，V0.2 把「自己的多台设备」连成统一文件空间——但两者都要求设备在同一个二层网络（mDNS 可达）。V0.3 要**突破局域网**：不在同一网络的已配对设备之间也能连接同步、发送，并支持把文件**分享**给指定好友 / 设备。

四个目标：

1. **远程可达**：任意网络环境下已配对设备可互联——先保证可达（中继兜底），再追求更快（直连 / 打洞）。
2. **端到端加密不变**：无论直连还是中继，内容始终在设备间 TLS 1.3 + 证书固定加密，任何中间节点（包括你自己的服务器）看不到内容。
3. **远程能力**：把 V0.2 的跨设备索引交换 + 按需拉取、以及 V0.1 的 AA 发送，跑到广域网链路上（复用 ATP，不新增数据语义）。
4. **分享链接**：生成带权限 / 过期的分享，指定好友或设备可取，不做社区分享。

设计原则延续 [AGENTS.md](AGENTS.md)：稳定 > 功能，简单 > 复杂，默认安全。**不引入账号体系**——设备身份始终是密钥对（V0.1 起）。

### 1.1 基础设施定位：仅自建、单进程（已确认）

**V0.3 只设计「自部署 `aa4c-server`」，不假设官方公益节点。** `aa4c-server` 是**一个二进制、一个进程**，同时承担信令（Rendezvous：注册 / 查询 / 打洞信令）与中继（Relay：盲转发）两个职责——合并的直接收益是中继凭证在进程内校验，不需要任何跨服务认证设计。

- 用户在自己的 VPS / NAS / 家用机上跑一个 `aa4c-server`，把它的地址填进客户端设置。
- 自己的多台设备填**同一个**地址；好友设备各用各的服务器（寻址规则见 §3.4）。
- 官方公益节点、多节点联邦、节点发现等留到 V0.3 之后再评估。
- 服务器代码与客户端同仓库（`crates/aa4c-server`），随 release 发布 Linux 二进制 + Dockerfile。

## 2. 连接阶梯（ICE-like）（✅ 已实现，里程碑 C1–C5）

设备要与某台**已配对**设备建立连接时，依次尝试，任一成功即停止（呼应 [PROTOCOL.md](PROTOCOL.md) §12）：

| 顺序 | 方式 | 说明 |
|------|------|------|
| 1 | **局域网直连** | mDNS 发现（同 V0.1），同网优先，零依赖、最快 |
| 2 | **公网直连** | 对端 endpoints 里有可达公网地址（有公网 IP / 已做端口映射）时 QUIC 直连 |
| 3 | **UDP 打洞** | 经服务器信令交换反射地址（自建反射端点，见 §3.2），双向同时发包打洞，成功后 QUIC 直连 |
| 4 | **Relay 中继** | 打洞失败：双方各自连 `aa4c-server`，服务器盲转发端到端加密字节流 |

- 只对 **`trusted=1` 的已配对设备**发起远程连接；陌生设备不进入此流程。
- 每次连接记录「用了哪一档」（`ConnectionVia::Direct`/`Punch`/`Relay`，里程碑 C4/C5），UI 只暴露「直连 / 中继（较慢）」两种人话——打洞（`Punch`）在展示上并入「直连」，见 §10。
- **优雅降级**：未配置服务器或服务器不可达时，退回纯局域网行为（同 V0.2），不报错阻塞。
- **实现顺序**（见 §11）：第 4 档（中继）先于第 3 档（打洞）落地——中继确定性可达、可在 CI 测试；打洞是提速优化，失败不损可用性。**测试环境提示**：回环/CI 环境没有真实 NAT，打洞会稳定成功并抢在中继之前——专门验证中继的测试需要显式关闭打洞（`TransferConfig::disable_punch`，里程碑 C5 教训）。

## 3. `aa4c-server`：信令职责（Rendezvous）

### 3.1 服务器身份：密钥对 + 地址内指纹（与设备同构）

服务器与设备用**同一套信任模型**：Ed25519 密钥对 + 自签名 TLS 证书，**不依赖 CA / 域名 / Let's Encrypt**。

- 服务器地址格式：**`aa4c://host:port#<证书指纹前16位hex>`**。
- 客户端建立 TLS 连接时校验服务器证书指纹与地址中的 pin 一致，不一致立即断开。
- 地址（含指纹）通过设置页填写、配对流程或分享链接携带传播——拿到地址即拿到信任锚点，无 TOFU 窗口。

### 3.2 信令协议：帧层 bincode 长连接（不用 HTTP/WS）

客户端与服务器之间是**一条 TLS 长连接**，复用既有帧层（4 字节大端长度 + bincode），消息族为独立的 `ServerMessage` enum（与设备间 `Message` 分离、独立版本号 `server_proto`，同样遵守「只追加变体」约束）。**不引入 HTTP / WebSocket / axum 依赖**——客户端与服务器全是自有 Rust 代码，技术栈保持 tokio + rustls。

消息职责（字段在里程碑 2 于 [PROTOCOL.md](PROTOCOL.md) Part C 定稿）：

| 消息 | 方向 | 职责 |
|------|------|------|
| `SrvHello` / `SrvHelloAck` | C↔S | 协商 `server_proto`，服务器下发能力（含中继端点信息） |
| `Challenge` / `ChallengeReply` | S↔C | 服务器发 nonce，客户端私钥签名回执——**身份验证不依赖时钟** |
| `Register` | C→S | 注册本设备候选端点（本地地址 + 反射地址）+ `proto`/版本 + **已配对设备允许名单**；周期性续约（TTL） |
| `Lookup` / `LookupReply` | C→S→C | 查询目标设备端点；服务器校验查询方已过挑战、且其 id 在**目标的允许名单**内 |
| `Signal` / `IncomingSignal` | C→S→C | 打洞候选交换：`Signal` 在发起方自己的常驻连接上发出，服务器把它转成 `IncomingSignal` 推给目标的常驻连接——**已实现（里程碑 C5）**，反射地址不经公共 STUN，由 `aa4c-server` 自带的轻量 QUIC 端点探测（同一进程、与 TCP 信令端口号相同，见 §4） |
| `RelayRequest` / `RelayGrant` | C→S | 申请中继会话，服务器发放一次性 `session_token`（进程内登记） |

**反射地址探测（里程碑 C5）**：`aa4c-server` 额外绑定一个轻量 QUIC 端点（与上面的 TCP 信令**同一个端口号**，UDP/TCP 命名空间互不冲突），不走 `ServerMessage` 协议——设备用自己**真正用于设备间 P2P 的那个 QUIC 端点**连一次，服务器把 `Connection::remote_address()`（即 NAT 之后观测到的源地址）经一条 uni 流回给它，这就是自建版的 STUN binding response。**必须用同一个本地端口**去探测：NAT 的外部映射通常按"本地端口"分配，换个端口探测出来的地址对后续真实打洞没有意义。不做身份鉴权（反射地址本身不敏感），接受任意合法 Ed25519 客户端证书即可。

### 3.3 查询授权：允许名单 + 挑战应答（取代「配对关系证明」）

初稿的「配对时互签 proof」有**吊销漏洞**（解除配对后对方仍持有永久有效的 proof），已弃用。改为：

- **注册时上传允许名单**：设备每次 `Register` 附带「当前已配对设备 id 列表」（服务器侧可哈希存储）。
- **查询时挑战应答**：查询方先过 `Challenge`（私钥签 nonce）证明自己是某 device_id，服务器再检查该 id 是否在目标的允许名单里，在才返回端点。
- **吊销自然发生**：解除配对后，下一次注册的名单里就没有对方——无需任何显式吊销协议。
- 服务器仍然只存「端点 + 名单」，**无文件元数据、无内容**；名单泄露的社交图信息在自建信任域内可接受。

### 3.4 寻址规则：查谁，去谁的 home server 查

每台设备的可达性锚定在**它自己配置的服务器**（home server）上：

- 自己的多台设备配同一个服务器 → 互查同一处，天然成立。
- 好友设备用他自己的服务器 → **向对端的 home server 发起 Lookup**；打洞信令也发生在**被叫方**的服务器上（被叫方与其 home server 保持长连接，信令可达）。
- 对端 home server 地址（含指纹）在**配对时交换并落库**，此后随注册续约刷新——这是 `devices` 表的正式字段（§8），不是可选项。
- 中继会话使用**被叫方**的服务器（它必然可达被叫方）。

## 4. `aa4c-server`：中继职责（Relay）

打洞失败时的兜底通道。中继**只盲转发加密字节流**，不解密、不理解 ATP。

```
RelayOpen   { session_token }      // token 由信令侧发放（同进程登记），中继不知道双方身份
RelayData   <opaque bytes>         // 端到端 TLS 之上，中继不可解密
RelayClose
```

- `session_token`：**一次性 + 短 TTL**（发放后未在窗口内建立即作废）；信令与中继同进程，校验即查表。
- 端到端仍是 TLS 1.3 + 证书固定（同 §5），中继是纯管道，看不到 device_id 与内容。
- 限速 / 配额由自建节点运营者（即用户自己）配置。
- 中继流量计入「连接质量」显示，UI 提示「通过中继，速度可能较慢」。
- **不承诺抗流量分析**（非目标，记入 [SECURITY.md](SECURITY.md) 威胁模型）。

## 5. 会话层：QUIC

- 广域网传输通道用 **QUIC**（quinn），TLS 1.3 内建，证书固定规则与 V0.1 §2 完全相同（对端证书公钥 BLAKE3 指纹 == 已记录 DeviceId）。
- **首版单流等价迁移**：在一条 bidi 流上原样运行既有 ATP 收发循环（V0.2 已把收发循环泛型化，可直接跑在任何 `AsyncRead + AsyncWrite` 上）——最小改动先跑通广域网链路。**单任务多流**（每文件独立流、并行与独立重传）作为后续优化，放在打洞之后（见 §11）。
- **v1/v2 TCP 通道保留**（局域网默认路径）；`Hello.proto` 协商取双方最小值，遇低版本对端自动降级（[PROTOCOL.md](PROTOCOL.md) §14）。QUIC 通道上握手协商 `proto ≥ 3`。
- **断点续传**：广域网链路不稳，续传是刚需。方案：接收方 accept 后回 `ResumeReport`（**新增追加变体**，不修改既有 `Offer`——bincode 追加兼容约束），报告 `.aa4c-part` 已落盘部分按块重算哈希得到的可信偏移，发送方从偏移续传。详见 [PROTOCOL.md](PROTOCOL.md) §13。
- QUIC 监听端口 = 现有 TCP 监听端口同号（UDP），不新增配置项。

## 6. 远程能力（复用，不新增数据语义）（✅ 已实现，里程碑 C4）

连接阶梯（§2）建立的是一条**已认证、端到端加密的双向通道**——上层能力直接复用 V0.1/V0.2 的协议：

- **远程发送**：V0.1 的 `Offer`/分块/`FileAck` 跑在连接阶梯解出的通道上（QUIC/TCP 直连或中继裸管道+叠加 mTLS，见 §2/§5），`send_files` 里程碑 C3 就已经接进阶梯。
- **远程同步**：V0.2 的索引摘要交换（`IndexRequest`/`IndexEntries`）+ 按需拉取（`FetchRequest` → 反转角色回推）里程碑 C4 起同样接入连接阶梯——完全信任设备即使不在同一局域网，也能交换索引、点黄拉取。此前 `sync_exchange`（跨设备索引交换的后台循环）只认 mDNS 在线快照，远程（跨网络）完全信任设备永远同步不到，是本里程碑补的缺口。
- **对端解析**：`resolve_peer`（发送/拉取用）与 `sync_exchange`（索引同步用）现在共用同一套「mDNS → 落库最后地址 → 对端 home server Lookup」阶梯（`aa4c-core::orchestrate::resolve_addr`），不再各自维护一份。
- **触发策略**：mDNS 的 `DeviceFound` 仍然即时触发一次索引同步（局域网设备上线反应快）；额外加一条**周期定时器**（30s，`sync_exchange::REMOTE_REFRESH_INTERVAL`）作为远程设备的兜底——`DeviceFound` 只对 mDNS 能发现的设备触发，远程设备永远不会产生这个事件。
- **在线判定**：`remote_index` 黄/红判定从「mDNS 在线」扩展为「mDNS 在线 **或** 最近一次远程索引同步仍在新鲜窗口内（90s，`orchestrate::REMOTE_INDEX_FRESH_WINDOW_MS`，约 3 倍周期定时器间隔）」——用"最近同步成功过"作为"当时确实可达"的证据，不必另起一次实时探测。注意这不是绝对保真（远程「新鲜」≠ 一定可达，NAT 变动等）——黄色条目拉取失败时给温和提示 + 可重试，不让黄色变成谎言。
- **连接质量**：`CoreEvent::TransferConnected{task_id, via}`（`via: direct|relay`）在出站连接（`dial()`）建立成功后立即广播一次，只有发起方（发送/拉取）收得到；只存于当次会话内存，不落库。设置页新增「远程连接」区块（服务器地址 + 开关），传输卡片按 `via` 显示「直连」/「中继（较慢）」徽标。

## 7. 分享链接（AA Share）

把文件 / 文件夹分享给**指定好友或设备**（非社区分享，见 [PROJECT_VISION.md](PROJECT_VISION.md) §产品边界）。

### 7.1 模型

- 一个分享 = `{ token, 目标(限定路径), 权限, 过期时间, 状态 }`，落 `shares` 表（§8）+ `share_access` 访问记录。
- `token`：**≥128 bit 熵**随机串（base58），**即能力（capability）**：持有有效 token 即可按其权限访问，不需要账号。
- **分享目标必须落在共享范围内（已索引内容）**：复用 V0.2 的 `resolve_shared` 解析与安全边界（绝不按任意路径读盘、天然防 `..` 穿越）。任意路径分享意味着全新的解析器与攻击面，**后置**。
- 权限：V0.3 首版**只读**；读写留字段余量、不实现。
- 过期：绝对时间，过期即拒绝；可手动吊销（`revoked`）。

### 7.2 链接格式与打开方式

- 链接格式：**`aa4c://share/<base58(payload)>`**，payload 含 `{ host device_id, token, host 的 server 地址(含指纹) }`——不含内容、不含密钥；配套二维码（移动端扫码）。
- 打开：AA 客户端识别链接 → 经 payload 里的服务器解析 host → 连接阶梯建连 → 出示 token → 按权限取文件。
- 局域网内可不依赖服务器（mDNS 直接找到 host）——**分享链接可先在局域网落地**，远程可达随连接阶梯就绪自然生效。
- ⚠️ 实现工作量提示：`aa4c://` 自定义 scheme 的 deep-link 注册涉及桌面三平台 + Android intent，是独立的平台适配工作，排期时单列。

### 7.3 与配对 / 信任的关系

- V0.3 首版**仅对已配对好友分享**（token 是「这次给你看这些」的范围限定，身份仍走配对信任）；匿名（未配对）分享后置，届时再定其鉴权模型。
- 好友打开分享即获得你的 server 地址提示（§3.4），无需事先手工同步。

## 8. 数据模型

落地表见 [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) §4c（待建表）。要点：

- `shares`：分享记录（`token` UNIQUE / 限定 `rel_path`（必须在共享范围内）/ 权限 / 过期 / 状态 / 创建时间）。
- `share_access`：访问记录（share_id / 访问方 device_id / 动作 / 时间）。
- `devices` 增列 `server_hint TEXT`：对端 home server 地址（含指纹），配对时交换、注册续约刷新（V0.3 迁移加列）。
- 连接配置存 `settings`（KV，复用现有表）：**`server_url`**（`aa4c://host:port#指纹`，单地址——中继端点由服务器 `SrvHelloAck` 下发，不单独配置）、**`enable_remote`**（总开关，**默认 `false`**，隐私优先：不配置、不打开就完全不出网）。

## 9. 安全考量（V0.3 新增面）

| 威胁 | 对策 |
|------|------|
| 服务器被攻破 / 作恶 | 只存端点映射 + 允许名单；端到端加密使其读不到内容；自建 = 你自己的信任域 |
| 服务器身份冒充 | 证书指纹写在地址里（`aa4c://…#fp`），连接即校验，无 TOFU 窗口 |
| DeviceId 枚举扫描 | Lookup 需挑战应答证明身份 + 在目标允许名单内 + 速率限制 |
| 解除配对后仍可查询 | 允许名单随每次注册刷新，吊销自然发生（无永久凭据） |
| 时钟漂移导致鉴权失败/重放 | 身份验证用 challenge-response（nonce），不依赖设备时钟 |
| 打洞信令伪造 | 信令经已挑战验证的长连接转发，消息由设备私钥签名 |
| 中继滥用（陌生人蹭带宽） | `session_token` 仅经信令发放（要求已配对 + 允许名单），一次性 + 短 TTL |
| 分享 token 泄露 | ≥128bit 熵 + 可过期 + 可吊销 + 默认只读 + 访问记录可审计；链接不含密钥/内容 |
| 中继流量分析 | 不承诺抵抗（非目标）；记入 [SECURITY.md](SECURITY.md) |

## 10. UI

见 [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md)：

- **设置页**新增「远程连接」：填服务器地址（一个）、总开关（默认关）；显示当前对端连接方式。
- **分享页（Share）**从「建设中」转为可用：生成分享（在共享范围内选文件/文件夹 + 过期）、管理分享（列表 / 吊销）、查看访问记录。
- 术语合规（[UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) §7）：不出现 NAT / STUN / Relay / QUIC / 打洞等技术词，只用「直连 / 中继（较慢）」这两个人话——`ConnectionVia::Punch`（打洞后升级成的直连，里程碑 C5）在 UI 上**并入「直连」**显示，不单独暴露成第三个词：打洞只是"怎么找到对方"的手段，一旦连上就是货真价实的直连，用户不需要关心过程（数据层仍保留三个取值，供未来需要时细分）。

## 11. 里程碑切分（已按评审调序：中继先于打洞）

详细实现拆解见 [V0.3_IMPLEMENTATION_PLAN.md](V0.3_IMPLEMENTATION_PLAN.md)。

1. ✅ **QUIC 会话层**：quinn + 证书固定复用 + 单流等价迁移 + `ResumeReport` 断点续传；两端手填地址即可验证，不依赖服务器。PROTOCOL 已定稿 proto=3 协商与流用法（§10/§13）。
2. ✅ **`aa4c-server` 信令**（`crates/aa4c-server`，只做信令面，中继面留给 C3）：注册（允许名单 + TTL 续约，覆盖式替换即吊销机制）/ 查询 / 客户端接入（`aa4c-core::server_link`，上线注册、`resolve_peer` 回落 Lookup）。鉴权复用 mTLS，未实现设计初稿的 `Challenge`/`ChallengeReply`（理由见 PROTOCOL.md §11）。`devices.server_hint` 列已建表，但配对协议尚未交换它——寻址目前只覆盖「自己的多台设备共用同一服务器」，跨服务器好友寻址留待后续（见 §12 表格与仍待实现列表）。PROTOCOL 已定稿 Part C（§11）。交付含 Dockerfile + `scripts/dev-server.sh` + release Linux 二进制。
3. ✅ **Relay 中继**（`crates/aa4c-server`，同进程加中继面）：`RelayRequest/Grant` 换一次性短 TTL token（8s）+ `RelayOpen/OpenAck` 撮合后**裸字节透明转发**（对设计稿 `RelayData`/`RelayClose` 的一处收敛，理由见 PROTOCOL.md §11/§12）；被叫方靠一条**常驻连接**收 `IncomingRelay` 推送（`aa4c-core::server_link::spawn_register_loop`，用 `Notify` 让设置变更立即生效，取代早期「一次性连接也发 Register」踩过的竞态坑——见下方表格）；`aa4c-transfer` 新增 `RelayDialer` 注入点（出站兜底）与 `accept_external`（入站接入统一分流）。连接阶梯「LAN → 公网直连 → 中继」贯通——**远程可用自此成立**（可发 preview）。e2e 覆盖：强制走中继完成一次真实文件传输、过期/复用 token 被拒绝。
4. ✅ **远程同步 / 发送**：`sync_exchange`（跨设备索引交换）与 `resolve_peer`（发送/拉取对端解析）改用同一套共享的解析阶梯（`orchestrate::resolve_addr`）；`fetch_index`/`fetch_file` 的 `addr` 参数改成 `Option<SocketAddr>`，解析不出地址（或直连失败）时同 `send()` 落中继兜底。`sync_exchange` 不再局限于 mDNS 在线快照——改为遍历全部完全信任配对设备，`DeviceFound` 即时触发之外新增 30s 周期定时器兜底远程设备。在线判定并入"最近一次远程索引同步是否新鲜"（90s 窗口）。连接质量：新增 `CoreEvent::TransferConnected{task_id, via: direct|relay}`，出站连接建立后广播一次（只存内存不落库）；前端设置页新增「远程连接」区块（服务器地址 + 开关），传输卡片按 `via` 显示「直连」/「中继（较慢）」徽标。e2e 覆盖：完全信任设备被逼到只剩中继一档时，索引同步与文件发送均能真实跑通。
5. ✅ **NAT 打洞**：反射地址探测（`aa4c-server` 自带轻量 QUIC 反射端点，见 §3.2，不依赖公共 STUN）+ `ServerMessage::Signal`/`IncomingSignal` 候选交换（发起方在自己的常驻连接上发出，回信作为推送收回，复用中继已有的 `pushable` 推送表）→ `aa4c-transfer` 新增 `PunchDialer` 注入点，`dial()` 插入连接阶梯第 3 档（直连失败之后、中继兜底之前）→ 候选地址逐个 `quic::connect`，第一个握手成功即为打洞直连（`ConnectionVia::Punch`）。回环环境没有真实 NAT，打洞会稳定成功——CI 验证的是"候选交换 + 反射探测 + 双向连接"这套接线本身，不是"真实穿透 NAT"（后者按计划留给人工双网络验证）；专门测中继的用例需要 `TransferConfig::disable_punch` 显式挡掉打洞（实现时真的踩到过"打洞把中继测试悄悄截胡"的教训，见下方表格）。视情况在此后做单任务多流优化。
6. **分享链接**：`shares`/`share_access` 表 + 生成 / 管理 / 吊销 / 访问记录 + token 鉴权 + deep-link（`aa4c://`）注册；先局域网落地。**可与 3–5 并行**。

## 12. 已确认的设计细节

| 议题 | 决定 | 落点 |
|------|------|------|
| 基础设施 | 仅自建，**单进程 `aa4c-server`**（信令+中继合一，token 进程内校验） | §1.1 / §4 |
| 服务器身份 | 密钥对 + 自签证书，**指纹写进地址** `aa4c://host:port#fp`，与设备同构、零 CA | §3.1 |
| 信令协议 | 复用帧层 bincode 的 TLS 长连接（`ServerMessage` 族，独立 `server_proto`），不用 HTTP/WS | §3.2 |
| 查询授权 | **允许名单（注册时覆盖式上传）**；初稿的互签 proof 因吊销漏洞弃用 | §3.3 |
| 寻址 | 查谁去谁的 home server；信令/中继用**被叫方**服务器；对端 server_hint 为 `devices` 正式字段（**已建表，尚未在配对时交换**，见下） | §3.4 |
| 身份验证（里程碑 2 实现收敛） | **复用 mTLS**，不实现设计初稿的 Challenge/ChallengeReply——TLS 握手本身已密码学证明身份，绑定在整条连接上，少一次往返、不引入独立签名依赖 | §3.2，PROTOCOL.md §11 |
| 注册 TTL | 60s（`aa4c_server::REGISTER_TTL`），客户端约每 TTL/3 续约；设置变更/解除配对立即触发一次续约，不等周期 | §3.2，`aa4c-core::server_link` |
| server_hint 交换范围（里程碑 2 缩小） | `resolve_peer` 目前只查**自己配置的服务器**，覆盖"自己的多台设备"主场景；跨服务器好友寻址需要配对协议交换 server_hint，而 `PairRequest`/`PairAccept`/`DeviceInfo` 是既有 bincode 结构体、追加字段会破坏 v1/v2 解码，需专门设计一条新消息，留待后续里程碑 | §3.4 |
| 会话层 | QUIC（quinn）证书固定复用；**首版单流等价迁移**，多流并行留作打洞后的优化 | §5 |
| 断点续传 | 接收方 accept 后回 `ResumeReport`（追加变体），**不修改既有 `Offer`** | §5 |
| 设置项 | 单 `server_url`（中继端点由服务器下发）+ `enable_remote` **默认关** | §8 |
| 分享范围 | 仅已配对好友、仅共享范围内已索引内容（复用 `resolve_shared` 边界）、默认只读 | §7 |
| 里程碑顺序 | **中继先于打洞**：打洞是提速优化不是可达前提；远程可用在里程碑 3 成立 | §11 |
| 账号体系 | 不引入，设备与服务器身份都是密钥对 | §1 / §3.1 |
| 能力复用 | 远程同步 / 发送直接复用 V0.2 索引交换 + V0.1 ATP，不新增数据语义 | §6 |
| 中继数据面（里程碑 3 实现收敛） | **不实现设计初稿的 `RelayData`/`RelayClose`**：`RelayOpen`+`RelayOpenAck` 撮合成功后连接直接转入裸字节透明转发，省去逐包重新编解码帧头；效果等价（服务器仍只盲转发字节、不解密），`session_token` 一次性——首次被 `RelayOpen` 触碰即从登记表移除，无论撮合成败 | §4，PROTOCOL.md §11/§12 |
| 中继 token TTL | 8s（`aa4c_server::RELAY_TOKEN_TTL`）：合法撮合只需几个 RTT，这个窗口只在对端确实不可达时才会被等满——越短失败越快，不拖累连接阶梯整体失败延迟 | §4 |
| 常驻连接与「立即生效」（里程碑 3） | `enable_remote=true` 时维持一条常驻连接周期续约 `Register` 并监听 `IncomingRelay` 推送；设置变更/解除配对用 `Notify` 唤醒**同一条**连接立刻重新注册，不再另开一次性连接——早期实现让一次性连接也发 `Register`，会与常驻连接抢服务器侧的推送登记槽位，一次性连接断开时的清理会把常驻连接刚登记好的活通道顶掉，直到下一轮周期续约（最长 TTL/3）才恢复，这段窗口内的中继推送悄悄丢失（实测踩到的真实竞态） | §3.2/§3.4，`aa4c-core::server_link` |
| 中继会话授权边界 | `RelayRequest`/`RelayOpen` 本身不查允许名单（服务器不理解「配对」语义）；真正的安全边界在被叫方自己——中继裸管道撮合后跑的仍是设备间 mTLS + `trusted` 检查，未配对请求方在协议层被拒绝，和直连路径完全同构 | §4，PROTOCOL.md §15 |
| 对端解析收口（里程碑 C4） | `resolve_peer`（发送/拉取）与 `sync_exchange`（索引同步）共用同一套地址解析阶梯（`orchestrate::resolve_addr`），不再各自维护一份、容易跑偏 | §6，`aa4c-core::orchestrate` |
| 远程同步触发策略（里程碑 C4） | `DeviceFound` 即时触发（局域网）+ 30s 周期定时器兜底（远程设备不会产生 `DeviceFound`）；不做指数退避，个人自托管场景轮询开销可忽略 | §6，`aa4c-core::sync_exchange` |
| 在线判定新鲜窗口（里程碑 C4） | 90s（约 3 倍周期定时器间隔，容忍一次没赶上的周期）：最近一次远程索引同步成功即视为"当时确实可达"，不必另起一次实时探测；不是绝对保真，拉取失败给温和提示 + 可重试 | §6，`aa4c-core::orchestrate` |
| 连接质量上报范围（里程碑 C4） | 只报「直连/中继」（`ConnectionVia::Direct`/`Relay`），不做数据库持久化，事件只存当次会话内存——历史记录不含这个字段；`Punch` 取值预留给里程碑 C5 追加 | §6，`aa4c_types::event` |
| STUN 服务器来源（里程碑 C5 确定） | **`aa4c-server` 自带一个轻量 QUIC 反射端点兼做**，不引入公共 STUN 依赖——与设备用于 P2P 的**同一个** QUIC 端点连一次即可拿到反射地址，保证映射对后续打洞有意义（不同本地端口的映射通常不通用） | §3.2 |
| 打洞候选交换连接选择（里程碑 C5） | `Signal` 必须发在**发起方自己的常驻连接**上（不能用一次性连接）：对端的回信是另一条 `Signal`，服务器会把它当 `IncomingSignal` 推送**回同一条连接**——复用中继（C3）已有的 `pushable` 推送表，不需要新基础设施 | §3.2，`aa4c-core::server_link` |
| 打洞响应去重（里程碑 C5 教训） | 收到 `IncomingSignal` 时必须先判断"这是不是我自己在等的回信"——只有不是时才需要反向回信 + 打洞；不做这个区分会导致两边对同一次交换无休止地互相"回信"，是实现时真实踩到的死循环，不是假设风险 | `aa4c-core::server_link::SignalChannel` |
| QUIC 连接生命周期（里程碑 C5 教训，但影响面回溯到 C1） | 入站 QUIC 连接如果走的是"转交给钩子后立即返回"的分流分支（如 `IndexRequest`，钩子内部自己 `tokio::spawn` 不等它跑完），必须让 `quinn::Connection` 句柄和分离出去的读写流绑在一起活着——只传流、让本地 `Connection` 变量随分发函数返回而丢弃，钩子那个后台任务会在数据真正发出前就先撞见连接被拆（`Offer`/`FetchRequest` 因为全程 `.await` 到底不受影响）。同理，写完最后一条消息后不能立刻丢连接——写成功只代表进了本地发送缓冲区，不代表已送达，需要半关闭写侧再等对端也关闭它那侧 | `aa4c-transfer::quic::QuicDuplex`，`aa4c-core::dispatch::finish_write_side` |
| 打洞与中继的测试隔离（里程碑 C5 教训） | 回环/CI 环境没有真实 NAT，打洞一旦存在就会稳定成功并抢在中继之前——C3 那个"强制走中继"的测试在打洞加入后其实已经在悄悄测打洞。新增 `TransferConfig::disable_punch` 测试专用开关（同 `prefer_quic` 的先例），让验证中继/打洞的测试能各自确定性地隔离到自己的档位，并在测试里显式断言 `ConnectionVia` 而不只看"传输成功" | `aa4c-transfer::TransferConfig` |

仍待实现阶段细化：配对协议交换 `server_hint` 的具体消息设计（跨服务器好友寻址的前提）、打洞成功率的真实双网络验证（回环/CI 只能验证接线正确，不能验证真实穿透率）、单任务多流优化、匿名（未配对）分享的鉴权模型（后置）、读写分享（后置）、多服务器联邦（后置）、iOS 后台长连接受限的退化方案。
