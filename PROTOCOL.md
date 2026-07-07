# AA4C Protocol Specification

> AA 协议（AA Transfer Protocol，ATP）的权威规范。
> **Part A（proto v1，局域网）为 V0.1 实现标准**；**Part B（proto v2，广域网）为 V0.3 设计草案**，实现前可能调整。
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

## 11. 发现层升级：`aa4c-server` 信令（Part C，已实现信令面，里程碑 C2）

自建 **`aa4c-server`**（单进程，见 [CONNECT_DESIGN.md](CONNECT_DESIGN.md) §1.1；本里程碑只做
信令面，中继面 `RelayRequest`/`RelayGrant`/`RelayOpen`/`Signal` 留给 C3/C5 按同样的「只追加」
纪律加入）。服务器身份与设备同构：**Ed25519 密钥对 + 自签证书，证书指纹写进服务器地址**
（`aa4c://host:port#<指纹前16位hex>`，`aa4c_types::ServerAddr::parse` 解析），客户端连接时
校验对端证书指纹是否以此前缀开头，不依赖 CA / 域名。

客户端 ↔ 服务器为**一条 TLS 长连接**，复用本协议帧层（4 字节大端长度 + bincode；`encode_frame`
/`decode_body`/`read_message`/`write_message` 已泛型化，两套协议共用同一套帧实现）。消息族为
独立的 `ServerMessage` enum（`aa4c_proto::server`），独立 `server_proto` 版本，遵守「只追加
变体」。C2 已实现字段：

```rust
enum ServerMessage {
    SrvHello { server_proto: u16 },
    SrvHelloAck { server_proto: u16 },
    Register { endpoints: Vec<SocketAddr>, proto: u16, allow_list: Vec<DeviceId> },
    RegisterAck { ttl_secs: u64 },
    Lookup { device_id: DeviceId },
    LookupReply { endpoints: Vec<SocketAddr> },   // 未注册/已过期/不在名单内一律空列表
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
  TTL = 60s（`aa4c_server::REGISTER_TTL`），客户端约每 TTL/3 续约（`aa4c-core::server_link`
  的后台循环）；设置变更 / 解除配对会立即触发一次续约，不必等下一轮周期。全内存态，
  无持久化——进程重启即清空，客户端靠周期续约自愈。
- **查询授权 = mTLS 身份 + 允许名单**（初稿的「双方互签配对证明」因吊销漏洞弃用）：
  `Lookup` 只在目标设备**当前**登记的允许名单包含查询方时返回非空端点列表；**吊销自然
  发生**——覆盖式 `Register` 本身就是吊销机制，不需要任何显式吊销协议，下一次注册的名单
  里没有对方，查询立刻查不到。未注册 / 已过期 / 不在名单内**一律回空列表、不区分原因**，
  防止藉此探测 DeviceId 是否存在。
- **寻址规则**（C2 缩小范围）：`resolve_peer` 目前只向**自己配置的服务器**查询，覆盖
  「自己的多台设备天然共用同一服务器」这一主场景；`devices.server_hint`
  （对端 home server 地址）列已建表，但**配对协议尚未交换这个字段**——`PairRequest`/
  `PairAccept`/`DeviceInfo` 是既有 bincode 结构体变体，追加字段会破坏 v1/v2 解码，需要
  一条新的追加消息才能安全传递，留给后续里程碑（随连接阶梯/打洞一起补齐）。跨服务器的
  好友寻址在此之前不可用，是已知、有意缩小的范围（见 HANDOFF.md）。

## 12. 连接建立顺序（ICE-like）

依次尝试，任一成功即停止：

1. **局域网直连**：mDNS 发现（同 v1）
2. **公网直连**：对端 endpoints 中有可达公网地址
3. **UDP 打洞**：通过 Rendezvous 信令交换 STUN 探测到的反射地址，双向同时发包打洞，成功后升级 QUIC
4. **Relay 中继**：双方各自连接 Relay，Relay 盲转发加密字节流

Relay 协议：

```
RelayOpen   { session_token }      // token 由信令阶段协商，Relay 不知道双方身份
RelayData   <opaque bytes>         // 端到端 TLS，Relay 不可解密
RelayClose
```

- Relay 限速与配额由自建节点运营者配置；`session_token` 一次性 + 短 TTL，由信令侧发放、进程内校验
- 中继流量计入"连接质量"显示，UI 提示"通过中继传输，速度可能较慢"

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
| 服务器作恶/被攻破 | 只存端点映射 + 允许名单；端到端加密使其无法读取内容；自建=自有信任域 |
| 服务器身份冒充 | 证书指纹写进服务器地址（`aa4c://…#fp`），连接即校验，无 TOFU 窗口 |
| DeviceId 枚举扫描 | Lookup 需挑战应答证明身份 + 在目标允许名单内；速率限制 |
| 解除配对后仍可查询 | 允许名单随每次注册刷新，吊销自然发生（无永久凭据） |
| 时钟漂移 / 重放 | 身份验证用 challenge-response（nonce），不依赖设备时钟 |
| 打洞信令伪造 | 信令经已挑战验证的长连接转发，消息由设备私钥签名 |
| 中继滥用（陌生人蹭带宽） | session_token 仅经信令发放（已配对 + 允许名单），一次性 + 短 TTL |
| Relay 流量分析 | 不承诺抗流量分析（非目标）；记录在 SECURITY.md 威胁模型 |

详细威胁模型见 [SECURITY.md](SECURITY.md)。
