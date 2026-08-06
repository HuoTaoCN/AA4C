# AA4C — AA for Connection

**Connect all your devices.**

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Android-lightgrey.svg)](#platform-support)
[![Release](https://img.shields.io/badge/release-v0.5.0--preview-green.svg)](https://github.com/HuoTaoCN/AA4C/releases)

> [中文](README.md) · English

AA4C (**AA for Connection**) is an open-source, cross-platform **device connection platform**. Connect your devices first — file transfer, sync, sharing, downloading and AI-assisted archiving then follow naturally. The core is always **device-to-device connection**.

> AA this file to me.
> AA the photos to my desktop.
> AA the model to the server.

AA4C is **not** a download manager, a BitTorrent client, a cloud drive, a sync tool, or a community platform. It stands for one thing: **your data belongs to you**.

---

## Contents

- [Why AA4C](#why-aa4c)
- [What makes it different](#what-makes-it-different)
- [Five capabilities](#five-capabilities)
- [Platform support](#platform-support)
- [Install and get started](#install-and-get-started)
- [Open source · Open · Secure](#open-source--open--secure)
- [Documentation](#documentation)
- [Build from source](#build-from-source)
- [Status and roadmap](#status-and-roadmap)
- [Contributing](#contributing)

---

## Why AA4C

The usual way — your data takes a detour through a third party:

```
Phone → Cloud drive (third-party server) → Desktop
```

AA4C — devices talk directly, the data stays yours:

```
Phone ↔ Desktop ↔ NAS ↔ Server
```

| Principle | What it means |
|-----------|---------------|
| **Peer-to-peer** | Devices transfer directly, with no third-party server in the path |
| **Local-first** | Data lives on your own devices; nothing is forced into a cloud |
| **Encrypted by default** | All traffic is TLS 1.3 end-to-end encrypted, and encryption cannot be switched off |
| **You are in control** | No accounts, no cloud, no subscription — and no outbound traffic at all unless you configure it |

### Why "AA"?

AA is a verb: **Send · Sync · Share · Archive**. It is the core action once devices are connected, and we would like it to become a new verb in the world of devices.

**4C = for Connection** (4 ≈ for, C = Connection). Everything is built around connection: **connection first, features second**.

---

## What makes it different

These are the things AA4C genuinely does differently.

### 🔌 One app, five capabilities

Normally you would run LocalSend for transfers, Syncthing for sync, Motrix or qBittorrent for downloads, and sort files by hand. AA4C puts **transfer, sync, share, download and archive** into one app with one device identity and one task center — a finished download can be auto-archived, and the archived file syncs to your NAS, without switching tools.

### 🌐 Zero outbound traffic unless you ask for it

Remote connectivity is **off by default**. Leave it off and AA4C only ever talks on your LAN. There is no telemetry, no crash reporting, no "anonymous statistics", no account system — you will not find an analytics SDK or a third-party endpoint anywhere in the source.

### 🏠 Signaling and relay are **self-hosted only**

When cross-network connections need a signaling or relay server, AA4C ships **no official public nodes**. You deploy your own `aa4c-server` (a single binary, or Docker). Your path stays yours — there is no "the vendor shut down the server" or "the vendor was asked to hand over data" scenario. The server only ever sees ciphertext and endpoint mappings, never file contents.

### 🟢 Metadata-first sync: your disk survives

Traditional sync tools drop a full copy of every folder on every device. AA4C syncs **only names and directory structure** by default and fetches content on demand. Each file is colored by availability:

| State | Meaning |
|-------|---------|
| 🟢 Local | Content is already on this machine — just open it |
| 🟡 Fetchable | Content is on an online device — one click retrieves it |
| 🔴 Offline | Only an offline device has it; unavailable for now |

A 1 TB media library can appear in full on a thin laptop while occupying a few MB of index.

### 🛡️ Graded device trust, not "paired means everything"

Pairing is not a blank cheque. Trust has four levels, and only **your own devices**, which you explicitly promote to *Full trust*, take part in cross-device indexing and sync:

| Level | Typical peer | Cross-device index / sync | Receiving files |
|-------|--------------|---------------------------|-----------------|
| Full trust | Your own devices | ✅ Two-way | Can be set to auto-accept |
| Friend | Friends / family / team | ❌ Manual shares only | Confirmation required |
| Temporary | One-off exchange | ❌ | Confirmation required each time |
| Unknown | Discovered, not paired | ❌ | Rejected |

Pairing lands on *Friend* by default — **least privilege out of the box**.

### 🤖 Fully local AI, and "rules act, AI only suggests"

AI archiving runs on a local llama.cpp (`llama-server`) instance against your own GGUF model files. **No cloud calls at all** — it works offline. The engine starts lazily and stops when idle, so it does not sit on your RAM.

The permission boundary matters more: **only deterministic rules may move files; AI output always lands in a pending-confirmation queue and never touches your files on its own.** When the AI is wrong, the worst outcome is a suggestion you ignore.

### 📚 Ask questions about your own files

Point the local knowledge base at a directory and AA4C indexes it locally. You can then ask questions and get answers with citations back to the source text. Your documents never leave the machine.

### 🧠 A downloader that understands model files

When you download something like `Qwen3-4B-Q4_K_M.gguf`, AA4C **parses the GGUF header itself** (reading only the first few dozen KB — never the tensor data) to identify architecture, parameter size, quantization and context length, then files it into your model library and syncs it across devices. If you run models locally, no other download manager does this for you.

### ⚖️ A clean license boundary

aria2 and Transmission are GPL software. AA4C **neither copies their source nor links their libraries** — they run as separate child processes, reached only over **loopback (127.0.0.1) with a secret regenerated at every start**. That is how AA4C itself stays Apache-2.0 and remains safe for companies to integrate.

### ↩️ Every automatic action is reversible

Anything auto-archiving moved can be undone from *Archive → Recent actions*. Automation has to be reversible; otherwise it is making irreversible decisions on the user's behalf.

---

## Five capabilities

| Capability | What it does | Status |
|------------|--------------|--------|
| **AA Send** | LAN device discovery plus peer-to-peer encrypted transfer for files, folders and very large files, with resume | ✅ V0.1 |
| **AA Sync** | Multi-device folder sync: metadata-first, on-demand fetch, live file watching, conflicts kept side by side | ✅ V0.2 |
| **AA Share** | Wide-area connectivity (QUIC + NAT hole punching + self-hosted relay) and share links for chosen friends, family or team | ✅ V0.3 |
| **AA Download** | Unified download center: HTTP / HTTPS / FTP (aria2) plus BitTorrent / magnet (Transmission) | ✅ V0.4 |
| **AA Archive & AI** | Rule-based auto-classification and archiving, model library, AI tag suggestions, local knowledge base — all cloud-free | ✅ V0.5 |

Step-by-step usage for each is in the [User Guide](docs/en/USER_GUIDE.md).

---

## Platform support

| Type | OS | Stack | Status |
|------|----|-------|--------|
| Desktop | Windows / macOS / Linux | Tauri 2 + Vue 3 | ✅ All capabilities |
| Mobile | Android | Tauri 2 (same codebase) | 🧪 Experimental — transfer and sync |
| Mobile | iOS / iPad | Tauri 2 (same codebase) | 📋 Planned |
| Server | Linux x86_64 (NAS / VPS) | `aa4c-server` single binary | ✅ Signaling + relay |

> The download center and AI archiving are desktop-only for now.

---

## Install and get started

### Install a release build (recommended)

Grab your platform's package from [Releases](https://github.com/HuoTaoCN/AA4C/releases):

| Platform | File | Notes |
|----------|------|-------|
| macOS | `.dmg` | Universal (Apple Silicon + Intel). Unsigned — right-click → Open on first launch, or allow it under System Settings → Privacy & Security |
| Windows | `.msi` | Double-click to install |
| Linux | `.deb` / `.rpm` / `.AppImage` | For AppImage, `chmod +x` then run |
| Android (experimental) | `.apk` | Requires "install from unknown sources" |

### Your first AA in three steps

1. Put both devices on the **same WiFi / LAN** and open AA4C — each shows up on the other's home screen
2. Click **Pair** on the other device's card and confirm that the **6-digit code matches on both screens**
3. Go to *Transfer*: pick files → pick a device → hit AA

To let two devices sync with each other, choose "Yes, this is my device" in the pairing success dialog to promote the peer to **Full trust**.

### Devices not showing up? Check the firewall first

AA4C uses **TCP 42420** (transfer) and **UDP 5353** (mDNS discovery) on the LAN:

- **Windows**: accept the firewall prompt on first run and make sure **Private networks** is ticked
- **macOS**: click **Allow** when asked whether to accept incoming connections
- **Linux (ufw)**: `sudo ufw allow 42420/tcp && sudo ufw allow 5353/udp`
- Many corporate and public networks enable client isolation or block multicast, which makes discovery impossible — try an ordinary home WiFi

More troubleshooting in the [FAQ](docs/en/FAQ.md).

---

## Open source · Open · Secure

None of this is a slogan; every item can be checked against the source. Details in [Open, Free and Secure](docs/en/OPEN_AND_SECURE.md).

### Open source

- **Apache License 2.0** — commercial use allowed, corporate integration allowed, patent grant included
- All code, build scripts, CI configuration and design documents are public; every release is built in the open by GitHub Actions on three platforms
- Dependencies are MIT / Apache-2.0 / BSD only; GPL components are strictly isolated as child processes over RPC, so they never affect the license

### Open

- **Open protocol**: the wire protocol, frame format, handshake and pairing flow are all written down in [PROTOCOL.md](PROTOCOL.md) — anyone can build a compatible client
- **Your data can leave**: metadata sits in a local SQLite database ([DATABASE_SCHEMA.md](DATABASE_SCHEMA.md)) and files are just files — no proprietary container, no encrypted jail. Uninstalling AA4C takes none of your data with it
- **Self-hostable infrastructure**: signaling and relay are self-deployment only — see the [self-hosting guide](docs/en/SELF_HOSTING.md)
- **Built to extend**: a layered Plugin API and open API are committed V1.0 goals ([ROADMAP.md](ROADMAP.md))

### Secure

| Layer | Mechanism |
|-------|-----------|
| Device identity | Ed25519 keypair; DeviceId = BLAKE3(public key), unforgeable |
| Establishing trust | Two-way 6-digit PIN, checked visually; **each side derives the PIN independently — it never crosses the network** |
| Channel encryption | TLS 1.3 with a self-signed certificate pinned to the device fingerprint (= DeviceId); no CA involved |
| Integrity | Per-file BLAKE3 verification with automatic retransmission on mismatch |
| Authorization | Only paired devices may initiate transfers; receiving requires confirmation by default; four trust levels |
| Key custody | Private keys stay local (file mode 0600), are never written to the database or logs, and never leave the device |
| Local engines | aria2 / Transmission / llama-server always bind 127.0.0.1 with a random secret and are never exposed to the LAN |

**Security features cannot be configured away** — project rules forbid adding any switch that disables encryption or verification.

The full threat model and vulnerability reporting process are in [SECURITY.md](SECURITY.md).

---

## Documentation

### For users

| Document | Contents |
|----------|----------|
| 📖 [User Guide](docs/en/USER_GUIDE.md) | Complete walkthrough of every feature (transfer / sync / share / download / archive / settings) |
| ❓ [FAQ and troubleshooting](docs/en/FAQ.md) | Devices not found, transfers failing, downloads stuck, AI unavailable… |
| 🖥️ [Self-hosting guide](docs/en/SELF_HOSTING.md) | Deploying `aa4c-server` for cross-network connectivity |
| 🔓 [Open, Free and Secure](docs/en/OPEN_AND_SECURE.md) | Privacy commitments, where data goes, license boundary, security model |

> 中文文档：[docs/](docs/)

### For developers

Core design documents are written in Chinese and live at the repository root:

| Document | Contents |
|----------|----------|
| [PROJECT_VISION.md](PROJECT_VISION.md) | Product and technical white paper |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Overall architecture and crate layout |
| [API_DESIGN.md](API_DESIGN.md) | Rust module interface design |
| [PROTOCOL.md](PROTOCOL.md) | AA wire protocol (LAN + WAN) |
| [DATABASE_SCHEMA.md](DATABASE_SCHEMA.md) | SQLite schema |
| [UI_DESIGN_SPEC.md](UI_DESIGN_SPEC.md) | UI and interaction specification |
| [TESTING.md](TESTING.md) | Test strategy and acceptance checklist |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contribution guide |
| [SECURITY.md](SECURITY.md) | Security policy and threat model |
| [CHANGELOG.md](CHANGELOG.md) | Changelog |

Per-module design: [SYNC_DESIGN.md](SYNC_DESIGN.md) · [CONNECT_DESIGN.md](CONNECT_DESIGN.md) · [DOWNLOAD_DESIGN.md](DOWNLOAD_DESIGN.md) · [ARCHIVE_DESIGN.md](ARCHIVE_DESIGN.md) · [TOUCH_DESIGN.md](TOUCH_DESIGN.md)

---

## Build from source

Requirements: Rust stable (≥ 1.85), Node.js ≥ 20, pnpm ≥ 9, Tauri CLI 2.x. Platform system dependencies are listed in the [Tauri docs](https://tauri.app/start/prerequisites/).

```bash
git clone https://github.com/HuoTaoCN/AA4C.git
cd AA4C
cargo test --workspace              # full Rust test suite
cd apps/desktop && pnpm install
pnpm test                           # frontend unit tests
pnpm tauri dev                      # run the desktop app in dev mode
pnpm tauri build                    # build an installer for this platform
```

Pre-commit check (CI runs the same on all three platforms):

```bash
cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

Environment setup notes and known pitfalls are in [HANDOFF.md](HANDOFF.md); conventions are in [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Status and roadmap

**Current release: v0.5.0-preview** — everything from V0.1 through V0.5 (transfer / sync / share / download / archive and AI) is implemented and shipped in the preview build.

| Version | Codename | Goal | Status |
|---------|----------|------|--------|
| V0.1 | Alpha | LAN discovery, pairing, file transfer | ✅ Released |
| V0.2 | Beta | Trust levels, cross-device index, continuous sync | ✅ Released |
| V0.3 | Connect | NAT traversal, self-hosted relay, share links | ✅ Released |
| V0.4 | Download | Unified download center (HTTP/FTP + BT/magnet) | ✅ Released |
| V0.5 | AI | Rule-based archiving, model library, AI suggestions, local knowledge base | ✅ Released (preview) |
| V0.6 | Touch / Direct | Tap-to-pair (NFC), off-grid connectivity (WiFi Direct / Bluetooth) | 📐 Design finalized, not yet implemented |
| V1.0 | Ecosystem | All platforms + plugin system + developer SDK | 📋 Planned |

Detailed scheduling is in [ROADMAP.md](ROADMAP.md).

### Explicitly out of scope

| Not doing | Why |
|-----------|-----|
| Community / content platform | Moderation and legal exposure are enormous, and it is not the goal |
| Resource or model marketplaces, file communities | No centralized content distribution |
| Centralized cloud storage | Your data belongs to you; we do not build central storage |
| Official public relay nodes | Infrastructure is self-hosted only, avoiding single points and data risk |

---

## Contributing

Code, documentation, tests, translations, issue reports and usability feedback are all welcome.

- Start with [CONTRIBUTING.md](CONTRIBUTING.md) and [PROJECT_VISION.md](PROJECT_VISION.md) to understand what AA4C is and is not
- Bugs and feature requests: [GitHub Issues](https://github.com/HuoTaoCN/AA4C/issues)
- Design discussion: [GitHub Discussions](https://github.com/HuoTaoCN/AA4C/discussions)
- **Do not open public issues for security vulnerabilities** — report privately per [SECURITY.md](SECURITY.md)
- Participation implies agreement with the [Code of Conduct](CODE_OF_CONDUCT.md)

## License

[Apache License 2.0](LICENSE)

## Community

- GitHub: https://github.com/HuoTaoCN/AA4C
- Website: https://aa4c.com
