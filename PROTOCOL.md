# AA4C Protocol Specification

> AA 协议（AA Transfer Protocol，ATP）的权威规范。
> **Part A（proto v1，局域网）为 V0.1 实现标准**；**Part B（proto v2+，广域网）V0.3 里程碑 C1–C6 已全部实现**，当前 `PROTO_VERSION=4`（见 §16）。
> [API_DESIGN.md](API_DESIGN.md) 中的协议片段是本文档的摘要，冲突时以本文档为准。

## 0. 总览

```
┌────────────────────────────────────────────┐
│  应用层    配对消息 / 传输消息（bincode）     │
├────────────────────────────────────────────┤
│  帧层      4 字节长度前缀 + 消息体           │
├────────────────────────────────────────────┤
│  安全层    TLS 1.3（自签名证书 + 证书固定）  │
├────────────────────────────────────────────┤
│  会话层    v1: TCP        v2: QUIC / Relay  │
├────────────────────────────────────────────┤
│  发现层    v1: mDNS       v2: Rendezvous    │
└────────────────────────────────────────────┘
```

| 常量 | 值 |
|------|-----|
| 协议版本（V0.1，局域网传输/配对） | `proto = 1` |
| 协议版本（V0.2，追加索引交换 / 按需拉取） | `proto = 2` |
| 协议版本（V0.3 里程碑 C1，QUIC 会话层 + 断点续传，**当前**） | `proto = 3` |
| 默认端口 | 42420（TCP；QUIC 同端口 UDP，Part B §10） |
| mDNS 服务类型 | `_aa4c._tcp.local.` |
| 最大帧长 | 16 MiB |
| 分块大小 | 4 MiB |
| 哈希 | BLAKE3 |
| 序列化 | bincode（小端，固定整数编码） |

---

# Part A — proto v1（局域网，V0.1 实现标准）

## 1. 发现层（mDNS）

每台设备注册 mDNS 服务并浏览同类服务：

- **服务类型**：`_aa4c._tcp.local.`
- **实例名**：`<device_id 前 16 位 hex>`
- **端口**：实际 TLS 监听端口（默认 42420，被占用时递增）

**TXT 记录**：

| key | 值 | 说明 |
|-----|-----|------|
| `id` | 64 位 hex | DeviceId = BLAKE3(Ed25519 公钥) |
| `name` | UTF-8 | 用户可见设备名 |
| `platform` | `windows/macos/linux/android/ios/server` | 平台 |
| `ver` | semver | AA4C 版本 |
| `proto` | `1` | 支持的最高协议版本 |

规则：

1. 收到 `id` 与本机相同的记录 → 忽略（自身回环）
2. 30 秒未刷新 → 判定离线
3. TXT 解析失败 → 忽略该设备（不报错）

## 2. 安全层（TLS 1.3 + 证书固定）

- 每台设备持有 Ed25519 密钥对，自签名 X.509 证书（rcgen 生成）
- **不使用 CA**。信任模型为证书固定（certificate pinning）：

| 场景 | 校验规则 |
|------|----------|
| 已配对设备间连接 | 对端证书公钥的 BLAKE3 指纹 **必须等于** 本地记录的 DeviceId |
| 配对（首次见面） | 暂不校验指纹，改为校验"证书公钥 == PairRequest 声明的公钥"，由双向 PIN 完成人工信任 |

- 最低 TLS 1.3；禁用会话票据复用之外的降级路径
- 私钥永不出设备、永不入日志

## 3. 帧层

```
+----------------+---------------------------+
| len: u32 (BE)  | body: bincode(Message)    |
+----------------+---------------------------+
```

- `len` 为 body 字节数，大端
- `len > 16 MiB` → 协议错误，立即断开
- `Chunk` 消息的特殊规则：帧体之后**紧跟** `Chunk.len` 字节的原始文件数据（不参与 bincode 编码，避免拷贝）

## 4. 消息定义

```rust
enum Message {
    // —— 握手 ——
    Hello    { proto: u16, device_id: DeviceId },
    HelloAck { proto: u16, device_id: DeviceId },

    // —— 配对 ——
    PairRequest { device: DeviceInfo, public_key: [u8; 32] },
    PairAccept  { device: DeviceInfo, public_key: [u8; 32] },  // 接收方同意进入 PIN 核对
    PairConfirm,                                               // 本端用户确认 PIN 一致
    PairReject  { reason: String },

    // —— 传输 ——
    Offer       { task_id: TaskId, files: Vec<FileMeta> },
    OfferAnswer { task_id: TaskId, accept: bool },
    Chunk       { file_index: u32, offset: u64, len: u32 },    // 后跟 len 字节原始数据
    FileDone    { file_index: u32, hash: String },             // 整文件 BLAKE3 hex
    FileAck     { file_index: u32, ok: bool },
    TaskDone    { task_id: TaskId },
    Cancel      { task_id: TaskId, reason: String },
}

struct FileMeta {
    rel_path: String,   // '/' 分隔；禁止 ".."、绝对路径、盘符
    size: u64,
}
```

## 5. 握手

任何连接建立后，发起方先发 `Hello`：

```
A → B : Hello    { proto: 1, device_id: A }
B → A : HelloAck { proto: 1, device_id: B }
```

校验规则（双方）：

1. `device_id` 必须与对端 TLS 证书指纹一致，否则断开
2. `proto` 取双方最小值；无共同版本 → 断开
3. 用途为传输时：对端必须在本地 `devices` 表且 `trusted = 1`，否则回 `Cancel{reason:"not_paired"}` 并断开
4. 用途为配对时：跳过规则 3，进入配对流程

## 6. 配对流程（状态机）

```
A（发起方）                          B（接收方）
   │── Hello / HelloAck ──────────────│
   │── PairRequest{A info, pkA} ─────▶│  事件: PairingRequest → 用户接受
   │◀───── PairAccept{B info, pkB} ───│
   │                                  │
   │   双方计算 PIN 并显示（见 §6.1）  │
   │                                  │
   │── PairConfirm ──────────────────▶│  （A 用户确认后）
   │◀───────────────── PairConfirm ───│  （B 用户确认后）
   │   双方写库 trusted=1，配对完成     │
```

任一步骤可被 `PairReject` 终止；**60 秒**无下一步消息 → 双方超时失败。

### 6.1 PIN 推导

```
pin = u32::from_le_bytes( BLAKE3( min(pkA,pkB) ‖ max(pkA,pkB) )[0..4] ) % 1_000_000
```

- 6 位十进制，左侧补零；`min/max` 按 32 字节公钥字典序，保证双方计算结果一致
- PIN 由两端独立计算、独立显示，**不经网络传输**；中间人无法同时伪造两端 PIN（公钥不同 → PIN 不同）

## 7. 传输流程（状态机）

```
发送方 S                              接收方 R
   │── Hello / HelloAck（校验 trusted）──│
   │── Offer{task, files[]} ───────────▶│  事件: TransferRequest → 用户接受
   │◀──────── OfferAnswer{accept} ──────│  （拒绝 → 任务 Rejected，断开）
   │                                    │
   │  对每个文件 i（按 index 升序）：     │
   │── Chunk{i, off, len} + 原始数据 ──▶│  落盘 <save>/<rel_path>.aa4c-part
   │── …（顺序分块，直到文件结束）……───▶│  边写边算 BLAKE3
   │── FileDone{i, hash} ──────────────▶│  校验哈希
   │◀──────────── FileAck{i, ok} ───────│  ok → 重命名为正式文件
   │                                    │
   │── TaskDone ───────────────────────▶│  任务 Done，双方写库
```

规则：

1. **顺序传输**：v1 单连接、按文件顺序、分块顺序发送（实现简单，局域网带宽足够）
2. `FileAck{ok:false}` → 发送方重传该文件，**最多 2 次**；仍失败 → `Cancel`，任务 `Failed`
3. 任一方可随时发 `Cancel`；收到后清理 `.aa4c-part` 临时文件
4. 连接意外断开 → 双方任务 `Failed`（v1 不做断点恢复；恢复机制见 Part B §13）
5. 接收方对 `rel_path` 强制净化：拒绝 `..`、绝对路径、保留字符；重名自动追加 ` (1)`

## 8. 错误处理总则

| 情形 | 行为 |
|------|------|
| 帧超长 / bincode 解码失败 | 视为协议攻击，立即断开，不回复 |
| 未知消息变体 | 断开（v1 不做向前兼容跳过；版本协商保证不会出现） |
| 超时（任何等待 ≥ 60s） | 任务/会话失败 |
| 哈希不匹配 | 重传 ≤ 2 次后失败 |

## 8b. V0.2 同步扩展消息（proto 2；v1 之上向后兼容追加）

V0.2 把握手协议版本升到 `proto = 2`，并在 `Message` 末尾**追加** enum 变体（不改既有判别号）。
仅在**完全信任**设备之间使用，详见 [SYNC_DESIGN.md](SYNC_DESIGN.md)。

**版本门槛（发起方 gate）**：握手 `Hello.proto` 取双方最小值协商；发起方只在协商结果
`≥ 2` 时才发送这些 v2 消息。对端为 v1（老版本 / v0.1.x）时协商降到 1，发起方**直接不发**
索引/拉取消息（优雅降级为纯 v1 传输），而非靠对端 bincode 解码失败断开来兜底——后者仍作为
最后防线（旧版收到未知变体会解码失败并断开）。落点：`aa4c-types::SYNC_PROTO_VERSION`、
`TransferService::fetch_index` 与 `fetch.rs` 的握手后判断。mDNS TXT 的 `proto` 字段也随之
广播为 `2`，供未来提前跳过无效尝试（当前仍以握手协商为准）。

```rust
enum Message {
    // …§4 既有变体…
    IndexRequest,                                    // 拉索引摘要（里程碑 3）
    IndexEntries { entries: Vec<IndexItem>, last: bool },
    FetchRequest { rel_path: String },               // 按需拉取一个共享文件（里程碑 4）
}
struct IndexItem { rel_path: String, size: u64, hash: Option<String> }
```

- **索引交换**：A 握手后发 `IndexRequest`；B 校验对端为 full，分批回 `IndexEntries`
  （每批 ≤1000 条，`last=true` 收尾；非 full / 无共享回空批次，不泄露文件名）。
- **按需拉取**：A 握手后发 `FetchRequest{rel_path}`（限定展示路径）；B 校验 full + 路径落在
  共享范围内，**反转角色**用 §7 的 `Offer`→`Chunk`→`FileDone`→`FileAck`→`TaskDone` 回推内容，
  A 自动 `OfferAnswer{accept:true}`。解析失败回 `Cancel{reason:"not_shared"}`。不新增数据通路。

---

# Part B — 广域网 QUIC/Relay（proto ≥ 3）

> §10（QUIC 会话层）与 §13（断点续传）**已实现**（里程碑 C1，`PROTO_VERSION = 3`）；
> §11（信令）/ §12（连接阶梯）/ §15 其余部分仍是设计草案，随 C2 起的里程碑落地。
> 原则：**广域网版是既有协议的超集**，帧层、消息层、安全层不变，只扩展会话层与发现层。
> 历史备注：早期草案曾把这套广域网能力称作「proto v2」；V0.2 已把 LAN 内的握手版本占用为 `proto = 2`（索引/拉取），故广域网演进顺延到 `proto ≥ 3`，本文其余处的「v2」按此语境理解。
> 应用层设计（连接阶梯、自建信令/中继、远程能力复用、分享链接）见 [CONNECT_DESIGN.md](CONNECT_DESIGN.md)。
> **V0.3 已确认：Rendezvous / Relay 仅自建**，下文提到的「官方公益节点」不在 V0.3 范围，留待更后续评估。

## 9. 目标

- 不在同一局域网的已配对设备之间直接传输与同步
- 优先 P2P 直连（打洞），失败时回退 Relay 中继
- 不引入账号体系：设备身份依旧是密钥对，Rendezvous 服务器只做"电话簿"，看不到内容

## 10. 会话层升级：QUIC（已实现，里程碑 C1）

- 广域网传输通道改用 **QUIC**（quinn），TLS 1.3 内建，证书固定规则与 §2 相同；UDP 端口与 TCP 监听端口同号，`start_listener` best-effort 绑定（失败只警告，回落纯 TCP）
- **首版单流等价迁移**（已实现）：每次逻辑会话开一条新 QUIC 连接 + 一条 bidi 流，`tokio::io::join` 拼成 `AsyncRead+AsyncWrite`，既有 ATP 收发循环零改动直接复用；**单任务多流**（每文件独立流、并行与独立重传）留作打洞落地后的性能优化
- v1/v2 TCP 通道保留作为局域网路径；QUIC 通道上握手协商 `proto = 3`；出站是否走 QUIC 由 `TransferConfig.prefer_quic` 控制（里程碑 C1 仅作测试/联调开关，默认 `false` 不影响任何现有行为；「按可达性自动选择」的正式逻辑收口在里程碑 C4）
- **keep-alive + 空闲超时**（`aa4c-transfer::quic::transport_config`）：2s 心跳 + 8s 空闲超时——应用层等待用户确认可长达 60s，心跳持续续命，只有心跳本身也送不出去的真断连才会在约 8s 内被两端各自发现

## 11. 发现层升级：`aa4c-server` 信令 + 中继 + 打洞（Part C，已实现信令面/中继面/打洞面，里程碑 C2+C3+C5）

自建 **`aa4c-server`**（单进程，见 [CONNECT_DESIGN.md](CONNECT_DESIGN.md) §1.1）。服务器身份
与设备同构：**Ed25519 密钥对 + 自签证书，证书指纹写进服务器地址**（`aa4c://host:port#<指纹
前16位hex>`，`aa4c_types::ServerAddr::parse` 解析），客户端连接时校验对端证书指纹是否以此
前缀开头，不依赖 CA / 域名。

客户端 ↔ 服务器为**一条 TLS 长连接**，复用本协议帧层（4 字节大端长度 + bincode；`encode_frame`
/`decode_body`/`read_message`/`write_message` 已泛型化，两套协议共用同一套帧实现）。消息族为
独立的 `ServerMessage` enum（`aa4c_proto::server`），独立 `server_proto` 版本，遵守「只追加
变体」。已实现字段（C2 信令 + C3 中继）：

```rust
enum ServerMessage {
    SrvHello { server_proto: u16 },
    SrvHelloAck { server_proto: u16 },
    Register { endpoints: Vec<SocketAddr>, proto: u16, allow_list: Vec<DeviceId> },
    RegisterAck { ttl_secs: u64 },
    Lookup { device_id: DeviceId },
    LookupReply { endpoints: Vec<SocketAddr> },   // 未注册/已过期/不在名单内一律空列表

    // —— 里程碑 C3：中继面 ——
    RelayRequest { target: DeviceId },                       // C→S：申请到 target 的中继会话
    RelayGrant { session_token: String, ttl_secs: u64 },     // S→C：一次性短 TTL token
    IncomingRelay { session_token: String, from: DeviceId }, // S→C：推给被叫方的常驻连接
    RelayOpen { session_token: String },   // C→S（新连接）：凭 token 开中继数据面
    RelayOpenAck { ok: bool },              // S→C：撮合结果，ok=true 后转入裸字节透明转发

    // —— 里程碑 C5：打洞候选交换 ——
    Signal { target: DeviceId, candidates: Vec<SocketAddr> },      // C→S：在自己的常驻连接上发出
    IncomingSignal { from: DeviceId, candidates: Vec<SocketAddr> }, // S→C：推给目标的常驻连接
}
```

- **身份验证复用 mTLS，不实现 `Challenge`/`ChallengeReply`**：这是对设计初稿的一处收敛。
  服务器接受任意合法 Ed25519 客户端证书（`tls_server_config(None)`，与设备间传输层同一套
  证书固定基础设施），TLS 握手本身已经密码学证明客户端持有其证书对应的私钥，身份绑定在
  **整条连接**上（比单条消息签一次 nonce 更强），且不依赖设备时钟——语义等价于设计稿要的
  安全属性，少一次往返、不必新增独立于 TLS 的签名依赖。查询方身份 = 其 mTLS 客户端证书
  指纹，不在消息里重复携带。
- **注册**：`Register` 覆盖式整体替换该设备在服务器的登记（含端点与允许名单）；服务器
  额外把连接的观测源地址并入 `endpoints`（免 STUN 的反射地址，同机/同网时天然可用）。
  TTL = 60s（`aa4c_server::REGISTER_TTL`），客户端约每 TTL/3 续约。全内存态，无持久化——
  进程重启即清空，客户端靠周期续约自愈。
- **常驻连接（里程碑 C3）**：`enable_remote=true` 时 `aa4c-core::server_link` 维持**一条**
  长连接，在其上周期续约 `Register`，同时 `select!` 监听服务器推送的 `IncomingRelay`——
  这是被叫方能收到中继会话通知的前提（CONNECT_DESIGN.md §3.4）。设置变更 / 解除配对等
  「立即生效」场景通过 `tokio::sync::Notify` 唤醒这条连接立刻重新注册，**不再另开一次性
  连接**：早期实现试过让一次性连接也发 `Register`，会与常驻连接抢 `pushable` 登记槽位——
  一次性连接发完消息就断开，若它抢到了槽位，断开时的清理会把常驻连接刚登记好的活通道
  顶掉，直到下一轮周期续约（最长 TTL/3）才能恢复，这段窗口内的中继推送会悄悄丢失（实测
  踩到的真实竞态，不是假设）。现在从根上只有一条连接会调用 `Register`，不存在竞争。
- **中继面（里程碑 C3，连接阶梯第 4 档）**：`RelayRequest` 换一次性 token（`RELAY_TOKEN_TTL`
  = 8s，`aa4c_server::RELAY_TOKEN_TTL`）；服务器 best-effort 把 `IncomingRelay` 推给
  `target` 当前的常驻连接（找不到就静默，不区分「未开启远程」/「不在线」/「从未存在」，
  防探测）。双方各自在**新连接**上 `RelayOpen{token}`，服务器按 token 撮合（`oneshot`
  把先到者的连接交给后到者所在的任务），两侧都到齐才回 `RelayOpenAck{ok:true}`；此后连接
  **转入裸字节透明转发**（`tokio::io::copy_bidirectional`），不再是 `ServerMessage` 帧——
  这是对设计稿的一处收敛：设计稿把数据面写成独立的 `RelayOpen`/`RelayData`/`RelayClose`
  三个消息，本实现只留 `RelayOpen`+`RelayOpenAck`，撮合后直接裸转发，省掉逐包重新编解码
  帧头的开销，效果等价（服务器依然只盲转发字节、不解密、token 一次性——被首次 `RelayOpen`
  触碰即从登记表移除，无论撮合成败）。设备间 mTLS 在这条裸管道上原样握手（`TlsConnector`/
  `TlsAcceptor` 泛型于任意 `AsyncRead+AsyncWrite`，同 QUIC 复用既有收发循环的道理），之后
  是与直连完全相同的 ATP。Token TTL 选得短：合法撮合只需要几个 RTT，这个窗口只在对端确实
  不可达时才会被等满，越短失败越快，不拖累连接阶梯整体的失败延迟。
- **传输层接入**：`aa4c-transfer::TransferService::dial` 直连失败（或压根没解析出地址）时，
  若 Core 注入了 `RelayDialer`（`aa4c-core::server_link::RelayDialerImpl`）就落到中继；
  入站侧新增 `TransferService::accept_external`，把中继撮合好的裸管道接进与 TCP/QUIC 入站
  完全相同的 TLS-accept + 分流管线（`recv::run_incoming_external`）。
- **打洞面（里程碑 C5，连接阶梯第 3 档，排在中继之前）**：
  - **反射地址探测**：`aa4c-server` 额外绑定一个轻量 QUIC 端点（`aa4c_server::reflect`，
    与上面的 TCP 信令**同一个端口号**，ALPN 用 `aa4c-reflect` 与设备间传输 QUIC 区分）。
    设备用自己**真正用于 P2P 的那个 QUIC 端点**连一次，服务器把 `Connection::remote_address()`
    （NAT 之后观测到的源地址）经一条 uni 流原样回给它——自建版的 STUN binding response，
    不引入公共 STUN 依赖。必须用同一个本地端口探测：NAT 的外部映射通常按本地端口分配，
    换个端口的探测结果对后续真实打洞没有意义。不做身份鉴权（反射地址本身不敏感）。
  - **候选交换**：发起方在**自己的常驻连接**上发 `Signal{target, candidates}`（本地候选 +
    反射地址）；服务器盲转发给 target 当前的常驻连接，包成 `IncomingSignal{from, candidates}`
    推送过去——复用中继面已有的 `pushable` 推送表，不需要新基础设施。收到 `IncomingSignal`
    的一方要判断这条消息是不是自己正在等待的回信：**是**就转交给等待者；**不是**（即别人
    发起的打洞请求）才需要反向探测自己的候选、向对方候选打几个尽力而为的探测包、把自己的
    候选用同样的 `Signal` 回信——不做这个区分会导致两边对同一次交换无休止地互相"回信"
    （实现时真实踩到的死循环，不是假设风险）。
  - **打洞尝试**：发起方拿到候选后逐个 `quic::connect`，第一个握手成功即为打洞直连
    （`ConnectionVia::Punch`）；候选交换失败/超时、或所有候选都连不上，均落到中继兜底。
  - **测试注意**：回环/CI 环境没有真实 NAT，打洞会稳定成功——`TransferConfig::disable_punch`
    是测试/联调专用开关（同 `prefer_quic` 的先例），让想专门验证中继的测试能确定性地绕过
    打洞（早期没有这个开关时，C3 的中继测试在打洞加入后其实已经被悄悄截胡，见 CHANGELOG）。
- **查询授权 = mTLS 身份 + 允许名单**（初稿的「双方互签配对证明」因吊销漏洞弃用）：
  `Lookup` 只在目标设备**当前**登记的允许名单包含查询方时返回非空端点列表；**吊销自然
  发生**——覆盖式 `Register` 本身就是吊销机制，不需要任何显式吊销协议，下一次注册的名单
  里没有对方，查询立刻查不到。未注册 / 已过期 / 不在名单内**一律回空列表、不区分原因**，
  防止藉此探测 DeviceId 是否存在。
- **寻址规则**：`resolve_addr`（`resolve_peer`/`sync_exchange` 共用的解析阶梯）除了向
  **自己配置的服务器**查询（覆盖「自己的多台设备天然共用同一服务器」这一场景）之外，
  还会查对端自己的 `server_hint` 服务器（`devices.server_hint` 列，配对时经 `PairServerHint`
  交换，proto ≥ 5，详见 §17）——两个用户各自搭独立服务器互为朋友时也能查到对方地址，
  这是对 C2 当初有意缩小范围的补完（见 HANDOFF.md）。**仍缩小的范围**：打洞/中继信令
  目前仍只会打向本机自己配置的服务器，跨服务器的信令联邦需要服务器间协议，是独立的、
  后置的项目（CONNECT_DESIGN.md §12「多服务器联邦」）。

## 12. 连接建立顺序（ICE-like）

依次尝试，任一成功即停止：

1. **局域网直连**：mDNS 发现（同 v1）
2. **公网直连**：对端 endpoints 中有可达公网地址
3. **UDP 打洞**（已实现，里程碑 C5）：通过自建反射端点探测反射地址，`Signal`/`IncomingSignal`
   交换候选，双向 `quic::connect` 打洞，成功后即为 QUIC 直连（`ConnectionVia::Punch`）
4. **Relay 中继**（已实现，里程碑 C3）：双方各自连接自建服务器，服务器盲转发加密字节流

Relay 协议（`ServerMessage`，见 §11；`aa4c_transfer::TransferService::dial` 落到这一档时的
入参 `addr` 可以是 `None`——第 1/2 档都没解析出地址也不阻断，先试打洞（第 3 档）再落中继）：

```
RelayOpen    { session_token }   // C→S（新连接），token 由 RelayRequest/RelayGrant 换来
RelayOpenAck { ok: bool }        // S→C：ok=true 后这条连接转入裸字节透明转发
```

- `ok=true` 之后不再有 `ServerMessage` 帧，纯字节直通对侧，直到任一方关闭连接（对应设计稿
  设想的 `RelayClose`——这里是隐式的：EOF/错误自然终止转发，见 §11 收敛说明）。
- Relay 限速与配额由自建节点运营者配置；`session_token` 一次性 + 短 TTL，由信令侧发放、进程内校验
- 中继流量计入"连接质量"显示，UI 提示"通过中继传输，速度可能较慢"——已实现（里程碑 C4）：
  `CoreEvent::TransferConnected{task_id, via: direct|relay}` 在 `dial()` 成功后广播一次
  （只有发起方——发送/拉取——收得到），只存当次会话内存，不落库；设置页新增「远程连接」
  区块（服务器地址 + 开关），传输卡片按 `via` 显示徽标

## 13. 断点续传（proto ≥ 3，已实现，里程碑 C1）

**不修改既有 `Offer` 变体**（bincode enum 只允许追加，改字段会破 v1/v2 解码）。新增追加变体：

```rust
// 接收方在 OfferAnswer{accept:true} 之后紧跟发送：
ResumeReport {
    task_id: TaskId,
    progress: Vec<FileProgress>,   // { file_index, verified_bytes }
}
```

- **确定性交换**（不是尝试性的）：双方协商 `proto ≥ 3` 时，接收方**必定**紧跟发送这条消息
  （哪怕 `progress` 为空）；发送方**必定**等待读取。proto < 3 的对端两边都不发送/不等待，
  行为与旧版完全一致。
- **`verified_bytes` 的计算**：接收方把已落盘的 `.aa4c-part` 长度向下截断到 4 MiB 边界
  （`aa4c_types::CHUNK_SIZE`），只信任**完整写入过的整块**，丢弃末尾不足一块的余量（可能是
  上次中断时的半截写入）——不做逐块签名比对，安全性来自「最终 `FileDone` 仍会对整个文件重新
  校验完整哈希」，前缀只要是真被完整写入过就绝对正确。
- 发送方对相应文件从 `verified_bytes` 处续传：重新流式读取源文件同样长度的前缀喂 BLAKE3
  hasher（保持整文件哈希正确），从该偏移开始才真正发送 `Chunk`（offset 非零）。接收方同理
  重新流式读取已落盘前缀喂 hasher，从偏移续写（不截断 part 文件）。
- **只有「明确取消」才清理 `.aa4c-part`**（本地用户取消 / 对端主动发 `Cancel`，PROTOCOL.md
  §7 规则 3 的原意）；网络掉线、超时等**意外**中断保留 part 文件——这正是续传的前提。孤儿
  part 文件的过期清理不在本里程碑范围内。
- proto < 3 通道不发送此消息，行为与 v1/v2 完全一致

## 14. 版本协商与兼容

- `Hello.proto` 协商取双方最小值；高版本端连低版本端自动降级为低版本行为（已在 V0.2 落地：
  proto 2 端连 proto 1 端时不发索引/拉取消息，见 §8b）
- mDNS TXT 的 `proto` 字段提前告知能力，避免无效尝试
- 新增消息只允许追加 enum 变体；后续引入 capability flags 做细粒度协商

## 15. 安全考量（广域网新增面）

| 威胁 | 对策 |
|------|------|
| 服务器作恶/被攻破 | 只存端点映射 + 允许名单 + 短 TTL 中继 token；端到端加密使其无法读取内容；自建=自有信任域 |
| 服务器身份冒充 | 证书指纹写进服务器地址（`aa4c://…#fp`），连接即校验，无 TOFU 窗口 |
| DeviceId 枚举扫描 | 身份验证用 mTLS（见 §11），Lookup 需在目标允许名单内才返回非空端点；未注册/不在名单/已过期一律回空列表、不区分原因 |
| 解除配对后仍可查询 | 允许名单随每次注册刷新，吊销自然发生（无永久凭据） |
| 时钟漂移 / 重放 | 身份验证复用 mTLS 握手本身的密钥持有证明，不依赖设备时钟、不需要 nonce（见 §11 的 mTLS 收敛说明） |
| 打洞信令伪造 | 留给里程碑 C5：设计上信令经已验证身份的常驻连接转发，消息由设备私钥签名 |
| 中继会话被冒领 | `RelayRequest`/`RelayOpen` 本身不检查允许名单（服务器不理解「配对」语义，见 §11）——真正的安全边界在**被叫方自己**：中继裸管道撮合后跑的仍是设备间 mTLS + `dispatch_shared` 的 `trusted` 检查，未配对的请求方在协议层被 `NotPaired` 拒绝，和直连路径完全同构。中继只是换了一层承载，不改变谁能真正建立会话 |
| 中继滥用（陌生人蹭带宽/占用连接） | `session_token` 一次性 + 短 TTL（8s，见 §11/§12），被首次 `RelayOpen` 触碰即从服务器登记表移除；限速/配额由自建节点运营者配置 |
| Relay 流量分析 | 不承诺抗流量分析（非目标）；记录在 SECURITY.md 威胁模型 |

详细威胁模型见 [SECURITY.md](SECURITY.md)。

## 16. 分享链接（`ShareRequest`，proto ≥ 4，已实现，里程碑 C6）

```
ShareRequest { token: String }   // C→S，打开分享链接的一方发
```

- 追加在 `Message` 末尾（`PROTO_VERSION` 3→4），不影响既有变体判别号。
- **鉴权不看 `trusted`**：分发时直接把 `token` 交给 Core 注入的 `ShareResolver`（校验
  `shares` 表：`status='open'` 且未过期），不检查请求方是否已配对——token 本身就是完整的
  访问凭证（CONNECT_DESIGN.md §7.1/§7.3）。这是对设计初稿"仍需配对信任"的收敛：token
  已经是 capability，再叠加配对要求是语义冲突。
- 校验通过后**反转角色**，复用 `send::serve_fetch`（与 `FetchRequest` 完全同一套：
  `Offer` → 分块 → `FileDone`/`FileAck` → `TaskDone`），不新增数据通路；解析失败/token
  无效统一回 `Cancel{reason:"invalid_or_expired_token"}`，不区分「不存在」/「过期」/
  「已吊销」（同 Lookup 的防探测惯例）。
- **实现时发现并修复的真实 bug**：`transfer_tasks.peer_device_id` 有外键约束
  `REFERENCES devices(id)`，此前所有消息类型都要求 `trusted`（peer 必然是已配对设备），
  这个假设从未被打破过。`ShareRequest` 允许未配对设备发起请求后，`serve_fetch`（服务端）
  与 `fetch::drive`（客户端）里原本无条件的 `insert_task`/`update_task_status` 会直接
  违反外键，把连接在协议中途悄悄挂断（表现为对端「connection lost」/「peer closed
  connection without sending TLS close_notify」，排查耗时较长——错误发生在对端，本地只看到
  连接异常关闭，看不到真正原因）。修法：两处都先查一次 `store.get_device(peer_id)`，
  已知设备才落库；未知设备跳过任务记录，只走 `share_access` 审计（协议本身不受影响）。
  详见 DATABASE_SCHEMA.md §4c.1、CONNECT_DESIGN.md §7.3/§12。

## 17. 配对时交换 server_hint（`PairServerHint`，proto ≥ 5，已实现，V0.3 遗留 gap 补完）

```
A（发起方）                          B（接收方）
   │── Hello / HelloAck ─────────────│  proto = min(A, B)
   │── PairRequest ──────────────────▶│
   │◀───── PairAccept ────────────────│
   │── PairConfirm ──────────────────▶│  （PIN 确认后，同 §6）
   │◀───────────────── PairConfirm ───│
   │                                  │  proto ≥ 5 时才继续，否则直接写库、流程结束
   │── PairServerHint{可选,自己的} ───▶│
   │◀──── PairServerHint{可选,对方} ───│
   │        双方写库 devices.server_hint，配对完成
```

**不修改既有 `PairRequest`/`PairAccept`**（同 §13 `ResumeReport` 的既有原则）：它们携带的
`DeviceInfo` 是 bincode 位置编码的既有结构体，直接加字段会让所有已配对过的旧版本客户端
解码失败（bincode 不是自描述格式，不能像 protobuf 那样跳过未知字段）。改为追加一个新变体：

```rust
PairServerHint {
    server_hint: Option<String>,
}
```

- **确定性交换**（不是尝试性的）：双方协商 `proto ≥ 5`（`SERVER_HINT_PROTO_VERSION`）时，
  在 §6 状态机的双向 `PairConfirm` 互相确认**之后**、写库**之前**，两端都**必定**发送这条
  消息（哪怕 `server_hint` 为空）并等待读取对方那条；proto < 5 的一方根本不认识这个变体，
  两端都不发送，行为与旧版完全一致（同 §13 的 gate 惯例）。
- **`server_hint` 的值**：发送方自己当前配置的 home server 地址（`Settings.server_url`），
  仅当 `Settings.enable_remote = true` 时才非空——语义与 `orchestrate::share_link()` 生成
  分享链接时决定要不要带 `host_server` 完全一致（未开启远程时不暗示一个没在实际生效的
  服务器）。
- **写库时机与覆盖规则**：收到的 `server_hint`（可能是 `None`）直接覆盖
  `devices.server_hint`，不做「保留旧值」的特殊处理——这次协商到的值就是最新事实。
  proto < 5（未协商）时才保留已存的旧值，行为等价于该字段从未被这次配对触碰过。
- **用途**：`aa4c-core::orchestrate::resolve_addr`（`resolve_peer`/`sync_exchange` 共用的
  地址解析阶梯，见 §12 表格「对端解析收口」条目）新增一档——查对端自己的 `server_hint`
  服务器，不要求本机也在那台服务器上注册过（`Lookup` 鉴权只看目标设备自己的允许名单，
  详见 §11/§15「DeviceId 枚举扫描」一行）。这条能解决「两个用户各自搭了独立
  `aa4c-server`」场景下互相找不到对方的问题（此前只覆盖「自己的多台设备共用同一服务器」
  这一种场景）。**仍是已知缩小范围**：这只解决「查到对方地址」，中继/打洞信令目前仍只会
  打向本机自己配置的服务器（两台互不知情的独立服务器之间没有公共撮合点，真正的跨服务器
  中继/打洞信令联邦需要服务器间协议，是独立的、后置的项目，见 CONNECT_DESIGN.md §12
  「多服务器联邦」）。
- **新鲜度承诺同 `last_addr`**：只在配对成功那一刻交换一次，之后不做持续刷新（配对之后
  一方改了 `server_url`，已配对的朋友不会自动收到通知）——`last_addr` 字段本来就是这个
  精度基准（`upsert_device` 只在配对时调用一次），`server_hint` 沿用同一惯例，不新增一条
  专门的刷新通道；需要更新时重新走一次配对流程即可。
