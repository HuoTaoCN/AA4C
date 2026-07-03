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
| 协议版本（V0.2，追加索引交换 / 按需拉取，**当前**） | `proto = 2` |
| 协议版本（V0.3 广域网 QUIC/Relay 草案） | `proto ≥ 3`（Part B，未定稿） |
| 默认端口 | 42420（TCP；广域网草案同端口 UDP 用于 QUIC/打洞） |
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

# Part B — 广域网 QUIC/Relay（proto ≥ 3，V0.3 设计草案）

> ⚠️ 以下为设计草案，V0.3 开发前需评审定稿。原则：**广域网版是既有协议的超集**，帧层、消息层、安全层不变，只扩展会话层与发现层；届时握手协议版本再上抬（`proto ≥ 3`）。
> 历史备注：早期草案曾把这套广域网能力称作「proto v2」；V0.2 已把 LAN 内的握手版本占用为 `proto = 2`（索引/拉取），故广域网演进顺延到 `proto ≥ 3`，本文其余处的「v2」按此语境理解。

## 9. 目标

- 不在同一局域网的已配对设备之间直接传输与同步
- 优先 P2P 直连（打洞），失败时回退 Relay 中继
- 不引入账号体系：设备身份依旧是密钥对，Rendezvous 服务器只做"电话簿"，看不到内容

## 10. 会话层升级：QUIC

- v2 传输通道改用 **QUIC**（quinn），TLS 1.3 内建，证书固定规则与 §2 相同
- 单任务多流：控制消息一条流，每个文件独立流（替代 §7 的顺序传输，支持并行与独立重传）
- v1 TCP 通道保留作为局域网回退

## 11. 发现层升级：Rendezvous 服务

设备通过 WSS 长连接注册到 Rendezvous 服务器（自部署或官方公益节点）：

```
POST register   { device_id, signed_at, signature, endpoints[] }
GET  lookup     { device_id } → { endpoints[], relay_hint }
WS   signal     （打洞信令转发：候选地址交换）
```

- 注册消息由设备私钥签名，服务器验签防止 DeviceId 抢注
- 服务器只存储 `device_id → 公网端点`，**无文件元数据、无内容、无社交关系**
- 查询限制：只能查询"已配对设备"（请求需携带双方配对关系证明：双方公钥的互签记录）

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

- Relay 限速与配额由节点运营者配置；官方节点默认 10 Mbps/会话
- 中继流量计入"连接质量"显示，UI 提示"通过中继传输，速度可能较慢"

## 13. 断点续传（v2）

`Offer` 扩展可选字段：

```rust
Offer {
    task_id, files,
    resume: Option<Vec<FileProgress>>,  // { file_index, verified_bytes }
}
```

- 接收方对 `.aa4c-part` 已落盘部分按 4 MiB 块重算哈希，回告可信偏移量
- 发送方从 `verified_bytes` 处续传

## 14. 版本协商与兼容

- `Hello.proto` 协商取双方最小值；高版本端连低版本端自动降级为低版本行为（已在 V0.2 落地：
  proto 2 端连 proto 1 端时不发索引/拉取消息，见 §8b）
- mDNS TXT 的 `proto` 字段提前告知能力，避免无效尝试
- 新增消息只允许追加 enum 变体；后续引入 capability flags 做细粒度协商

## 15. 安全考量（v2 新增面）

| 威胁 | 对策 |
|------|------|
| Rendezvous 服务器作恶/被攻破 | 只存端点映射；注册验签；端到端加密使其无法读取内容 |
| DeviceId 枚举扫描 | lookup 需配对关系证明；速率限制 |
| Relay 流量分析 | v2 不承诺抗流量分析（非目标）；记录在 SECURITY.md 威胁模型 |
| 打洞信令伪造 | 信令消息由设备私钥签名 |

详细威胁模型见 [SECURITY.md](SECURITY.md)。
