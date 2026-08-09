# Self-Hosting Guide (aa4c-server)

> [中文](../SELF_HOSTING.md) · [User Guide](USER_GUIDE.md) · [Project home](../../README.en.md)

If you only use AA4C on a LAN, you **need no server at all** — devices connect to each other directly.

You need one when devices on **different networks** should reach each other: the NAS at home and the laptop at work, or a share link sent to a friend in another city. That requires a machine both sides can reach, to handle **signaling** (finding each other) and, when hole punching fails, **relaying**. You deploy that machine yourself.

> **Read this first: you may not need a separate deployment.** Since V0.7 the desktop app
> ships an **embedded server**. If you have a machine at home that stays on — a desktop or a
> NAS — just turn on Settings → Remote → "Use this device as a relay". No extra machine to
> rent, none of the commands below.
>
> This document is for running a dedicated VPS, or running on a machine with no desktop app
> at all (a headless server, Docker, a container on a NAS). Both are identical on the wire,
> and clients use the same address format either way.
>
> **Either way, one requirement does not go away**: other devices have to be able to find the
> machine — a stable public address, or a DDNS hostname. That is the real boundary of "no
> third-party provider".


## Why self-hosted only?

AA4C ships **no official public nodes**. That is a deliberate product decision:

- **No single point of failure** — no "the vendor's server is down / unpaid / blocked" cutting off your devices
- **No data risk** — no "the operator was compelled to hand over data" scenario
- **The path is yours** — you can see where it runs, which ports are open, and what it logs

The cost is that you need a machine with a reachable address: a VPS, a home connection with a public IP, or a NAS behind port forwarding.

## What the server can see (important)

`aa4c-server`'s visibility is deliberately minimal:

| The server **can** see | The server **cannot** see |
|------------------------|---------------------------|
| Device IDs and their current network endpoints (IP:port) | File contents — traffic is end-to-end encrypted and the relay blindly forwards ciphertext |
| The "who may look me up" device-id list each device uploads at registration | File names, directory structure, any file metadata |
| Byte volume of relay sessions | The identities on either end of a relay (it only knows a one-time session token) |

The relay is a **dumb pipe**: it neither decrypts nor understands the AA4C protocol.

> Resistance to traffic analysis is not claimed — an observer can infer that two devices are exchanging data and how much, but not what. This is stated explicitly in the [threat model](../../SECURITY.md).

---

## 1. Prepare a machine

You need:

- Linux x86_64 (VPS, NAS, or a small box at home)
- An address **both devices can reach**: a public IP, a domain, or a tunneled address
- One open port (default **42420**, TCP)

## 2. Get the binary

### Option A: download the official build

Every release ships `aa4c-server_<version>_linux-x86_64`. From [Releases](https://github.com/HuoTaoCN/AA4C/releases):

```bash
chmod +x aa4c-server_v0.5.0-preview_linux-x86_64
mv aa4c-server_v0.5.0-preview_linux-x86_64 /usr/local/bin/aa4c-server
```

### Option B: build from source

```bash
git clone https://github.com/HuoTaoCN/AA4C.git
cd AA4C
cargo build --release -p aa4c-server
# output: target/release/aa4c-server
```

`aa4c-server` is a **single binary, single process** — no database, no runtime dependencies.

## 3. Run it

```bash
aa4c-server
```

Two environment variables configure it:

| Variable | Default | Meaning |
|----------|---------|---------|
| `AA4C_SERVER_DATA_DIR` | `./aa4c-server-data` | Identity directory. On first start the server generates its Ed25519 key and self-signed certificate here, and **keeps them from then on** |
| `AA4C_SERVER_LISTEN` | `[::]:42420` | Listen address and port. Dual-stack by default: one port serves both IPv6 and IPv4. Set `0.0.0.0:42420` explicitly for IPv4 only |

For example:

```bash
AA4C_SERVER_DATA_DIR=/var/lib/aa4c-server \
AA4C_SERVER_LISTEN=[::]:42420 \
aa4c-server
```

Log level is controlled by `RUST_LOG` (e.g. `RUST_LOG=debug`).

### Note your server address

At startup the log prints a line like:

```
把 aa4c://<你的可达地址>:42420#a1b2c3d4e5f6a7b8 填进客户端设置
(put aa4c://<your reachable address>:42420#a1b2c3d4e5f6a7b8 into client settings)
```

Replace the placeholder with the address clients can actually reach:

```
aa4c://your-server.example.com:42420#a1b2c3d4e5f6a7b8
```

> Everything after `#` is the **server's public key fingerprint**. Clients pin the certificate against it, so a wrong or hijacked address fails the handshake outright. **Store and share the full address through a channel you trust** — never just the hostname.

> ⚠️ **Persist `AA4C_SERVER_DATA_DIR`.** Losing it means a new identity, a new fingerprint, and every client has to be reconfigured. With Docker, always mount it as a volume.

## 4. Open the port

```bash
# ufw
sudo ufw allow 42420/tcp

# firewalld
sudo firewall-cmd --permanent --add-port=42420/tcp && sudo firewall-cmd --reload
```

On a cloud VM you must also allow the port in the provider's **security group / firewall rules** — this step is easy to forget.

## 5. Run it as a service (systemd)

Create `/etc/systemd/system/aa4c-server.service`:

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

# Hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/aa4c-server

[Install]
WantedBy=multi-user.target
```

Enable it:

```bash
sudo useradd --system --no-create-home aa4c
sudo mkdir -p /var/lib/aa4c-server && sudo chown aa4c:aa4c /var/lib/aa4c-server
sudo systemctl daemon-reload
sudo systemctl enable --now aa4c-server
sudo journalctl -u aa4c-server -f      # read the log to get your server address
```

## 6. Run it with Docker

No official image is published yet; a minimal Dockerfile does the job:

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
docker logs -f aa4c-server        # get your server address
```

> Do not omit `-v aa4c-server-data:/data` — the server identity lives there.

## 7. Configure the clients

Do this once on every device that needs cross-network connectivity:

1. Open AA4C → **Settings**
2. **Self-hosted server address**: paste the full `aa4c://host:port#fingerprint`
3. Turn on **Enable remote connectivity** (the toggle stays disabled until an address is set)

Once configured:

- On startup, the device **registers** its current network endpoint with the server and uploads a list of device ids allowed to look it up (your paired devices)
- To reach a device, it **queries** the server for that device's endpoint, then prefers a direct connection or NAT hole punching
- Only if that fails does it fall back to the **relay** — the UI notes that relayed connections may be slower

### Recommendation: point your own devices at one server

Using a single server address across your own devices is the simplest arrangement.

A friend's device can use their own server: **address lookups** are sent to the peer's server, whose address is exchanged and stored automatically during pairing. Be aware of one current limitation, though: **hole-punching and relay signaling still go only to the server this device has configured**. So in a cross-server setup where both sides may need relay fallback, the simplest arrangement remains **agreeing on one shared server**. Cross-server federation is a later milestone (see [CONNECT_DESIGN.md](../../CONNECT_DESIGN.md) §12).

## 8. How access control works

There is no allowlist file to maintain — authorization happens automatically:

1. At registration, a device uploads the **ids of devices it is currently paired with**
2. A device wanting to look it up must first **sign a random nonce** with its private key, proving it really is that device id
3. The server returns the endpoint only if that id appears in the target's allowlist

Which means: **unpairing = the next registration omits that peer = lookup access is gone**, with no explicit revocation protocol needed.

## 9. Operations

| Topic | Guidance |
|-------|----------|
| **Backups** | Back up `AA4C_SERVER_DATA_DIR`. Lose it and the identity changes, forcing every client to be reconfigured |
| **Upgrades** | Swap the binary and restart; the data directory is untouched and the fingerprint stays the same |
| **Bandwidth** | Direct connections and hole punching cost the server no bandwidth; only relayed sessions do. Rate limits and quotas are yours to configure at the system level (`tc`, provider-side shaping) |
| **Logging** | `RUST_LOG=info` is enough for day-to-day operation; logs contain no file contents or file names |
| **Ports** | Change `AA4C_SERVER_LISTEN` to move ports, then update the port in the client-side address |
| **IPv6** | IPv6 is already listened on by default (`[::]`, dual-stack). If your machine has a public IPv6 address, clients can often connect directly and skip hole punching and relaying entirely — just remember your firewall or security group has to open that port for IPv6 **as well**; IPv4 rules alone are not enough |

## 10. Troubleshooting

| Symptom | What to check |
|---------|---------------|
| Client configured but cannot connect | Is the process running? Is the port reachable (`nc -vz host 42420`)? Is the cloud security group open? |
| Handshake fails / fingerprint mismatch | Does the fingerprint after `#` match what the server currently logs? Recreating the data directory changes it |
| "Enable remote connectivity" toggle does nothing | You must fill in the server address first |
| Devices cannot find each other | Are both registered against the **same** server? Are they paired with each other (unpaired peers are not in the allowlist)? |
| Connected but slow | The session may be relayed. Relay is the fallback path and is bounded by your server's bandwidth |

---

## Related documents

- [CONNECT_DESIGN.md](../../CONNECT_DESIGN.md) — full design of signaling, relay, NAT traversal and share links
- [PROTOCOL.md](../../PROTOCOL.md) — wire protocol specification (parts B/C cover wide-area)
- [SECURITY.md](../../SECURITY.md) — threat model
- [Open, Free and Secure](OPEN_AND_SECURE.md) — where data goes and what is promised
