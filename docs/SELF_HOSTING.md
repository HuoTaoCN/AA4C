# 自建服务器指南（aa4c-server）

> [English](en/SELF_HOSTING.md) · [用户手册](USER_GUIDE.md) · [返回首页](../README.md)

只在局域网内用 AA4C 的话，**不需要任何服务器**——设备互相直连就够了。

当你想让**不在同一个网络**的设备互相连接（比如家里的 NAS 和公司的笔记本，或者给外地的朋友发分享链接）时，需要一台双方都能访问到的机器来做**信令**（互相找到对方）和必要时的**中继**（打洞失败的兜底通道）。这台机器由**你自己部署**。

> **先看这里：你可能不需要单独部署。** 从 V0.7 起，桌面端自带一个**内置服务器**——
> 家里那台常年开着的电脑或 NAS，在「设置 → 远程连接 → 让这台设备当中转站」里打开开关
> 就行，不用另外租机器、也不用跑下面这些命令。
>
> 本文档适用于：你想用一台独立的 VPS，或者要在没有桌面端的机器（纯服务器、Docker、
> NAS 上的容器）里跑。两种方式在协议上完全一样，客户端填的地址格式也一样。
>
> **无论哪种方式，都绕不开同一个前提**：别的设备得能找到这台机器——要么有固定公网地址，
> 要么配一个 DDNS 域名。这是「不依赖第三方服务商」的真实边界。


## 为什么只支持自建？

AA4C **不提供官方公共节点**，这是有意的产品决策：

- **没有单点风险**——不存在"官方服务器关停 / 欠费 / 被封"导致你的设备连不上
- **没有数据风险**——不存在"运营方哪天被要求交出数据"
- **链路在你手里**——你能看到它跑在哪、开了什么端口、留了什么日志

代价是需要你自己准备一台有公网可达地址的机器（VPS、有公网 IP 的家宽、或做了端口映射的 NAS）。

## 服务器能看到什么（重要）

`aa4c-server` 的可见范围被刻意限制到最小：

| 服务器**能**看到 | 服务器**看不到** |
|------------------|------------------|
| 设备 ID 与它当前的网络端点（IP:端口） | 文件内容——端到端 TLS 加密，中继只盲转发密文 |
| 每台设备注册时上传的"允许查询我的设备 id 列表" | 文件名、目录结构、任何文件元数据 |
| 中继会话的字节流量大小 | 中继两端的身份（中继只认一次性会话 token） |

中继是**纯管道**：它不解密、也不理解 AA4C 的传输协议。

> 不承诺抗流量分析——观察者能推断"有两台设备在传东西"和流量大小，但拿不到内容。这一点明确写在 [SECURITY.md](../SECURITY.md) 的威胁模型里。

---

## 一、准备一台机器

需要：

- Linux x86_64（VPS / NAS / 家里的小主机都行）
- 一个**双方设备都能访问到的地址**：公网 IP、域名，或内网穿透后的地址
- 开放一个端口（默认 **42420**，TCP）

## 二、获取二进制

### 方式 A：下载官方构建

每个 release 都附带 `aa4c-server_<版本>_linux-x86_64`，从 [Releases](https://github.com/HuoTaoCN/AA4C/releases) 下载：

```bash
chmod +x aa4c-server_v0.5.0-preview_linux-x86_64
mv aa4c-server_v0.5.0-preview_linux-x86_64 /usr/local/bin/aa4c-server
```

### 方式 B：自己从源码构建

```bash
git clone https://github.com/HuoTaoCN/AA4C.git
cd AA4C
cargo build --release -p aa4c-server
# 产物：target/release/aa4c-server
```

`aa4c-server` 是**单个二进制、单进程**，不依赖数据库、不依赖运行时。

## 三、启动

```bash
aa4c-server
```

用两个环境变量配置：

| 环境变量 | 默认值 | 说明 |
|----------|--------|------|
| `AA4C_SERVER_DATA_DIR` | `./aa4c-server-data` | 身份数据目录。首次启动会在这里生成服务器的 Ed25519 密钥与自签证书，**之后固定不变** |
| `AA4C_SERVER_LISTEN` | `[::]:42420` | 监听地址与端口。默认是**双栈**：同一个端口同时接受 IPv6 与 IPv4。要只听 IPv4 就显式写 `0.0.0.0:42420` |

例如：

```bash
AA4C_SERVER_DATA_DIR=/var/lib/aa4c-server \
AA4C_SERVER_LISTEN=[::]:42420 \
aa4c-server
```

日志级别用 `RUST_LOG` 控制（如 `RUST_LOG=debug`）。

### 记下你的服务器地址

启动后日志里会打印一行，形如：

```
把 aa4c://<你的可达地址>:42420#a1b2c3d4e5f6a7b8 填进客户端设置
```

把 `<你的可达地址>` 换成客户端真正能连到的地址（公网 IP 或域名），得到完整地址：

```
aa4c://your-server.example.com:42420#a1b2c3d4e5f6a7b8
```

> `#` 后面是**服务器公钥指纹**。客户端会拿它做证书固定——地址填错或被劫持，握手直接失败。**请通过可信渠道保存和传递这个完整地址**，不要只传主机名。

> ⚠️ **`AA4C_SERVER_DATA_DIR` 一定要持久化。** 目录丢了 = 服务器身份变了 = 指纹变了，所有客户端都要重新填地址。用 Docker 时务必挂载成卷。

## 四、开放端口

```bash
# ufw
sudo ufw allow 42420/tcp

# firewalld
sudo firewall-cmd --permanent --add-port=42420/tcp && sudo firewall-cmd --reload
```

云服务器还要在厂商控制台的**安全组 / 防火墙规则**里放行同一个端口——这一步经常被忘掉。

## 五、作为系统服务常驻（systemd）

创建 `/etc/systemd/system/aa4c-server.service`：

```ini
[Unit]
Description=AA4C signaling and relay server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=aa4c
Group=aa4c
Environment=AA4C_SERVER_DATA_DIR=/var/lib/aa4c-server
Environment=AA4C_SERVER_LISTEN=[::]:42420
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/aa4c-server
Restart=always
RestartSec=5

# 加固
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/aa4c-server

[Install]
WantedBy=multi-user.target
```

启用：

```bash
sudo useradd --system --no-create-home aa4c
sudo mkdir -p /var/lib/aa4c-server && sudo chown aa4c:aa4c /var/lib/aa4c-server
sudo systemctl daemon-reload
sudo systemctl enable --now aa4c-server
sudo journalctl -u aa4c-server -f      # 看日志，取服务器地址
```

## 六、用 Docker 跑

仓库暂未提供官方镜像，用一个最小 Dockerfile 即可：

```dockerfile
FROM rust:1.85 AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p aa4c-server

FROM debian:bookworm-slim
COPY --from=builder /src/target/release/aa4c-server /usr/local/bin/aa4c-server
ENV AA4C_SERVER_DATA_DIR=/data
ENV AA4C_SERVER_LISTEN=[::]:42420
VOLUME /data
EXPOSE 42420
ENTRYPOINT ["/usr/local/bin/aa4c-server"]
```

```bash
docker build -t aa4c-server .
docker run -d --name aa4c-server \
  -p 42420:42420 \
  -v aa4c-server-data:/data \
  --restart unless-stopped \
  aa4c-server
docker logs -f aa4c-server        # 取服务器地址
```

> `-v aa4c-server-data:/data` 不能省——服务器身份存在这里。

## 七、在客户端配置

每台需要跨网络连接的设备都要配一次：

1. 打开 AA4C →「设置」
2. **自建服务器地址**：填入完整的 `aa4c://主机:端口#指纹`
3. 打开**「开启远程连接」**开关（没填地址时这个开关是灰的）

配好之后：

- 设备启动时会向服务器**注册**自己当前的网络端点，同时上传一份「允许查询我的设备 id 列表」（就是你已配对的设备）
- 需要连某台设备时先向服务器**查询**它的端点，然后**优先直连 / NAT 打洞**
- 打洞失败才走**中继**——界面会提示「通过中继，速度可能较慢」

### 建议：自己的设备配同一台服务器

自己的多台设备配同一个服务器地址，互相查询最省事。

朋友的设备可以用他自己的服务器：**地址查询**会打向对方的服务器（对方的服务器地址在配对时自动交换并保存）。但要注意一个当前限制：**打洞和中继的信令目前仍只走本机自己配置的服务器**。所以跨服务器场景下，如果双方都需要中继兜底，最省事的做法仍是**双方约定使用同一台服务器**。跨服务器联邦是后续里程碑（见 [CONNECT_DESIGN.md](../CONNECT_DESIGN.md) §12）。

## 八、访问控制怎么生效

服务器不需要你维护什么白名单文件，授权是自动的：

1. 设备注册时，上传自己**当前已配对设备的 id 列表**
2. 别的设备来查询时，必须先**用私钥签一个随机数**证明自己确实是那个 device_id
3. 服务器检查这个 id 在不在目标设备的允许名单里，在才返回端点

所以：**解除配对 = 下一次注册的名单里就没有对方 = 自动失去查询权限**，不需要任何显式的吊销操作。

## 九、运维要点

| 事项 | 建议 |
|------|------|
| **备份** | 备份 `AA4C_SERVER_DATA_DIR`。丢了就换了身份，所有客户端都得重填地址 |
| **升级** | 换二进制重启即可，数据目录不动，指纹不变 |
| **带宽** | 直连和打洞不消耗服务器带宽；只有中继会走服务器流量。中继的限速与配额由你自己在系统层面配置（如 `tc`、云厂商限速） |
| **日志** | `RUST_LOG=info` 足够日常运维；日志不含文件内容与文件名 |
| **端口** | 想换端口就改 `AA4C_SERVER_LISTEN`，客户端地址里的端口跟着改 |
| **IPv6** | 默认已经在听 IPv6（`[::]` 双栈）。如果你的机器有公网 IPv6，客户端往往能直接连上、跳过打洞与中继——记得防火墙 / 安全组要**同时**放行 IPv6 的这个端口，只放 IPv4 规则是不够的 |

## 十、排查

| 现象 | 排查方向 |
|------|----------|
| 客户端填了地址但连不上 | 服务器进程在跑吗？端口通吗（`nc -vz 主机 42420`）？云厂商安全组放行了吗？ |
| 握手失败 / 指纹不匹配 | 地址里 `#` 后的指纹与服务器当前日志打印的一致吗？数据目录被重建过就会变 |
| 「开启远程连接」开关点不动 | 必须先填服务器地址 |
| 设备互相查不到 | 两台设备都注册到**同一台**服务器了吗？它们互相配过对吗（未配对不在允许名单里）？ |
| 连上了但很慢 | 可能走了中继。中继是兜底通道，速度受服务器带宽限制 |

---

## 相关文档

- [CONNECT_DESIGN.md](../CONNECT_DESIGN.md) —— 信令、中继、NAT 打洞、分享链接的完整设计
- [PROTOCOL.md](../PROTOCOL.md) —— 线路协议规范（Part B/C 是广域网部分）
- [SECURITY.md](../SECURITY.md) —— 威胁模型
- [《开源 · 开放 · 安全》](OPEN_AND_SECURE.md) —— 数据去向与隐私承诺
