# Open, Free and Secure

> [中文](../OPEN_AND_SECURE.md) · [Project home](../../README.en.md)

This document covers three things: what AA4C's open-source commitment actually is, what "open" concretely means here, and where your data really goes.

**Every claim below can be checked against the repository** — that is the point of writing them down. "We respect your privacy" is worth nothing; a statement you can verify is worth something.

## Contents

- [1. Open source](#1-open-source)
- [2. Open](#2-open)
- [3. Secure](#3-secure)
- [4. Privacy: where your data goes](#4-privacy-where-your-data-goes)
- [5. What we deliberately do not do](#5-what-we-deliberately-do-not-do)
- [6. How to verify all this yourself](#6-how-to-verify-all-this-yourself)

---

## 1. Open source

### License: Apache License 2.0

Apache-2.0 rather than GPL, so the project can actually be **used**:

- Commercial use, in-house corporate integration and closed-source derivatives are all permitted
- It **includes a patent grant** — contributors cannot turn around and sue users over patents
- It is compatible with almost every ecosystem

### Scope

Not "parts of the source"; all of it:

| Component | Location |
|-----------|----------|
| All application code | `crates/` (Rust core), `apps/desktop/` (Tauri + Vue frontend) |
| Server code | `crates/aa4c-server/` (signaling + relay) |
| Build and release pipelines | `.github/workflows/` (CI, release, engine packaging) |
| Engine fetch scripts | `scripts/fetch-engines.sh` (with checksums) |
| All design documents | Architecture / protocol / database / per-module docs at the repository root |
| Tests | Inline tests in each crate's `src/` plus integration tests in `tests/` |

**There is no community-vs-pro split, and no closed-source server component.**

### Builds are public and reproducible

Every release is built in the open by GitHub Actions on Windows, macOS and Linux. The pipeline configuration is in the repository and the logs are public. You can also distrust our packages entirely and build your own with `cargo build` and `pnpm tauri build` — the steps are in the [README](../../README.en.md#build-from-source).

### Dependency policy

- Dependencies are **MIT / Apache-2.0 / BSD** only
- New dependencies are checked against their advisory history, and CI runs `cargo audit`
- The project stays deliberately lean: if a few dozen lines of our own code will do, we do not add a dependency (GGUF header parsing and magic-byte detection are both hand-written)

### GPL isolation (and why it matters)

AA4C uses three external engines:

| Engine | Purpose | License |
|--------|---------|---------|
| aria2 | HTTP / HTTPS / FTP downloads | GPL-2.0 |
| Transmission | BitTorrent / magnet downloads | GPL-2.0 |
| llama.cpp (`llama-server`) | Local AI inference | MIT |

For the two GPL engines, AA4C:

- ❌ does **not** copy their source into the repository
- ❌ does **not** link against their libraries
- ✅ runs them as **separate child processes**, reachable only over **loopback (127.0.0.1) via RPC**

This is not a loophole; it is a clean process boundary, and it buys two practical things: **AA4C itself stays Apache-2.0** (safe for companies to integrate), and **engines can be swapped** — the BitTorrent engine was in fact switched from qBittorrent to Transmission with almost no impact on the layers above.

Contributors are held to the same rule: [CONTRIBUTING.md](../../CONTRIBUTING.md) forbids copying GPL / AGPL code into the repository.

---

## 2. Open

Open source only means "you can read the code". **Open** means you can leave, modify, and take over.

### 1. An open protocol

The wire protocol, frame format, handshake, pairing state machine, sync index exchange and share link format are all written down in [PROTOCOL.md](../../PROTOCOL.md), including version negotiation and compatibility rules.

**Anyone may implement a compatible client** — no permission, no certification programme.

### 2. Open data: you can walk away

The most practical guarantee. AA4C has **no data lock-in of any kind**:

| Data | Form |
|------|------|
| Received / downloaded / archived files | **Ordinary files** in ordinary directories you chose. No proprietary container, no encrypted wrapper, no chunked black box |
| Metadata (devices, tasks, index, rules, tags) | A local SQLite database, schema published in [DATABASE_SCHEMA.md](../../DATABASE_SCHEMA.md), openable with any SQLite tool |
| Configuration | Key-values in that same local database |

**Uninstalling AA4C takes none of your data with it**, and no "export wizard" is needed — the files were already there; take them with a file manager.

### 3. Open infrastructure: self-hosted only

The signaling and relay server needed for cross-network connectivity has **no official public nodes** — you deploy your own ([self-hosting guide](SELF_HOSTING.md)).

It looks like a missing convenience, but it buys:

- No "the vendor's server shut down / went unpaid / got blocked" cutting off your devices
- No "the operator was compelled to hand over data"
- Full visibility into which machine it runs on, which ports it opens, and what it logs

### 4. Built to extend

The layered Plugin API (Download / Sync / Share / AI / Storage / Notification) and an open API are committed V1.0 goals — see [ARCHITECTURE.md](../../ARCHITECTURE.md) and [ROADMAP.md](../../ROADMAP.md).

---

## 3. Secure

The complete threat model and reporting process are in [SECURITY.md](../../SECURITY.md); this is the summary.

### Design principles

**1. Encryption cannot be disabled.** Project rules forbid introducing any option that turns off encryption or verification. There is no "disable TLS for speed" switch.

**2. Least privilege by default.** Pairing lands on *Friend* and does not participate in indexing; receiving files requires confirmation; remote connectivity is off; every archive rule starts disabled. Every dangerous capability must be **explicitly enabled by you**.

**3. Automation must be reversible.** Anything auto-archiving moved can be undone in one click.

**4. AI holds no file-operation privileges.** Only deterministic rules move files; AI output always waits in a confirmation queue.

### Mechanisms

| Layer | Mechanism |
|-------|-----------|
| Device identity | Ed25519 keypair; DeviceId = BLAKE3(public key), unforgeable |
| Establishing trust | Two-way 6-digit PIN, visually compared. **Each side derives it independently and it never crosses the network** — a man-in-the-middle cannot make both sides agree |
| Channel encryption | TLS 1.3 with a self-signed certificate **pinned** to the device fingerprint (= DeviceId). No CA is involved, so a compromised CA ecosystem is irrelevant |
| Integrity | Per-file BLAKE3 verification with automatic retransmission on mismatch |
| Path safety | The receiver sanitizes relative paths, rejecting `..` and absolute paths (path traversal defense) |
| Protocol robustness | Frames are capped at 16 MiB and malformed frames drop the connection (protocol-level DoS defense) |
| Authorization | Only paired devices may initiate transfers; four trust levels bound indexing and sync |
| Key custody | Private keys live in a local file (mode 0600) and are **never in the database, never in logs, never off the device** |
| Local engines | aria2 / Transmission / llama-server bind `127.0.0.1` and authenticate with a **secret regenerated at every start** — unreachable from the LAN |
| Child processes | The direct-link engine reconnects and respawns automatically after a crash, and orphan protection stops stray processes surviving the app |

### Scope

**In scope**: LAN eavesdropping, man-in-the-middle, forged device identity, path traversal, transfers initiated by unpaired devices, tampering in transit, protocol-level DoS, a relay server trying to read content (end-to-end encryption means it only sees ciphertext).

**Explicitly out of scope**: a compromised device (malware, rooted OS), a user who skips the PIN check and pairs with an attacker, traffic analysis (an observer learns that two devices are exchanging data and how much, but not what), and physical access to an unlocked device.

Stating what is *not* covered matters as much as stating what is.

### Hard rules for contributors

- Changes touching keys, TLS verification or path handling **must include tests in both directions** (valid accepted / invalid rejected)
- Logging private keys, full public keys or file contents is forbidden
- No configuration option may disable a security feature

### Reporting a vulnerability

**Please do not open a public issue.** Report privately via [GitHub Security Advisory](https://github.com/HuoTaoCN/AA4C/security/advisories/new) or security@aa4c.com. The commitment: acknowledgement within 72 hours, initial assessment within 7 days, fix and disclosure targeted within 90 days, and public credit after the fix ships (unless you prefer to stay anonymous).

---

## 4. Privacy: where your data goes

### In one line: nowhere.

| Common worry | What actually happens |
|--------------|-----------------------|
| Are my files uploaded? | **No.** Devices connect directly on a LAN; across networks, traffic passes only through a server you deployed, as end-to-end encrypted ciphertext |
| Is usage data collected? | **No.** No telemetry, no crash reporting, no "anonymous usage statistics". There is no analytics SDK in the code |
| Is my file content read and analyzed? | **No.** AI runs entirely locally against your own model files, with zero cloud calls |
| Do I need an account? | **No.** There is no account system, no login, no cloud identity |
| Does it phone home when idle? | **No.** Remote connectivity is off by default, and while off AA4C stays on the LAN |
| Are there ads, promotions or content recommendations? | **No.** The download center provides, indexes and recommends nothing |
| Is my data still there after uninstalling? | **Yes.** They are plain files in directories you chose |

### The only case where AA4C reaches the internet on its own

Exactly one: **you filled in a server address and enabled remote connectivity**. AA4C then connects to **the server you deployed**, to register its endpoint and look up peers. There are no other outbound connections.

**No update checks, no remote configuration fetches, no automatic tracker-list syncing** — that last one is a deliberate trade-off. Comparable tools periodically pull public tracker lists from GitHub, which means the app regularly contacts a third-party address on its own. That conflicts with "no outbound traffic unless configured", so AA4C only supports pasting trackers in manually.

### What a self-hosted server can see

Even with remote connectivity on, the server's visibility is minimal:

- **Can see**: device IDs, their current network endpoints, the allowlist each device uploads, relay traffic volume
- **Cannot see**: file contents, file names, directory structure, any file metadata

The relay is a **dumb pipe**, blindly forwarding end-to-end encrypted bytes without decrypting or understanding the protocol.

---

## 5. What we deliberately do not do

Boundaries are part of the product:

| Not doing | Why |
|-----------|-----|
| Community / content platform | Moderation, abuse handling, legal exposure and running costs are enormous, and it is not the problem this project solves |
| Resource or model marketplaces, file communities | No centralized content distribution |
| Centralized cloud storage | Your data belongs to you; we do not build central storage |
| Official public relay nodes | Infrastructure is self-hosted only, avoiding single points and data risk |
| Telemetry and usage statistics | Even anonymized |
| Account system | The device identity *is* the identity; no centralized account needed |
| A switch to disable encryption | Security features are not configurable away |
| Letting AI act on files directly | AI may only suggest; nothing happens until the user confirms |

---

## 6. How to verify all this yourself

Do not take any of the above on faith — check it.

**Look for telemetry or third-party endpoints:**

```bash
grep -rniE "analytics|telemetry|sentry|amplitude|mixpanel|posthog" crates apps/desktop/src
grep -rnoE "https?://[a-z0-9.-]+" crates/*/src/*.rs
```

**Confirm the engines only bind loopback:**

```bash
grep -rn "rpc-bind-address\|LLAMA_ARG_HOST" crates/
```

**Inspect outbound behaviour**: read `crates/aa4c-core/src/server_link.rs` — the only outbound connection AA4C initiates **on its own behalf** (to the server you configured), gated by the `enable_remote` switch. All other outbound traffic is the direct result of something you asked for: a file you sent to a device, a download link you pasted.

**Inspect encryption and identity**: read `crates/aa4c-identity/` (Ed25519 keys, TLS certificate pinning, PIN derivation). Its tests cover both directions — matching fingerprints accepted, mismatched ones rejected.

**Inspect what is stored**: `DATABASE_SCHEMA.md` documents the tables, and you can open the local database with any SQLite tool.

**Build your own copy:**

```bash
cargo test --workspace
cd apps/desktop && pnpm install && pnpm test && pnpm tauri build
```

---

## Related documents

- [SECURITY.md](../../SECURITY.md) — full security policy and threat model
- [PROTOCOL.md](../../PROTOCOL.md) — wire protocol specification
- [DATABASE_SCHEMA.md](../../DATABASE_SCHEMA.md) — local database schema
- [Self-hosting guide](SELF_HOSTING.md) — taking over the infrastructure yourself
- [CONTRIBUTING.md](../../CONTRIBUTING.md) — license and security rules for contributors
