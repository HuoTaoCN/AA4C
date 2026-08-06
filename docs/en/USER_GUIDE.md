# AA4C User Guide

> Applies to v0.5.0-preview · [中文](../USER_GUIDE.md) · [Project home](../../README.en.md)

This guide walks through every feature of AA4C. If something is not working, check the [FAQ and troubleshooting](FAQ.md) first.

## Contents

1. [Installation](#1-installation)
2. [First run: pairing two devices](#2-first-run-pairing-two-devices)
3. [Interface overview](#3-interface-overview)
4. [Transfer: AA a file across](#4-transfer-aa-a-file-across)
5. [Sync: keep folders consistent across devices](#5-sync-keep-folders-consistent-across-devices)
6. [Share: send files to friends, family, colleagues](#6-share-send-files-to-friends-family-colleagues)
7. [Download: the unified download center](#7-download-the-unified-download-center)
8. [Archive and AI: auto-organizing, model library, knowledge base](#8-archive-and-ai-auto-organizing-model-library-knowledge-base)
9. [History](#9-history)
10. [Settings reference](#10-settings-reference)
11. [Where your data lives](#11-where-your-data-lives)

---

## 1. Installation

Download the package for your platform from [Releases](https://github.com/HuoTaoCN/AA4C/releases).

| Platform | File | Notes |
|----------|------|-------|
| Windows | `.msi` | Double-click to install. On first run, the firewall prompt appears — you **must** tick **Private networks**, otherwise devices cannot discover each other |
| macOS | `.dmg` | Drag to Applications. The build is **not notarized by Apple**, so on first launch right-click the icon → Open → Open. Alternatively allow it under System Settings → Privacy & Security |
| Linux | `.deb` / `.rpm` / `.AppImage` | Install deb/rpm with your package manager; for AppImage run `chmod +x AA4C*.AppImage` first |
| Android | `.apk` | Experimental. Requires "install from unknown sources" |

Packages are built by GitHub Actions in a public CI pipeline whose configuration lives in the repository — anyone can reproduce a build or build from source.

---

## 2. First run: pairing two devices

Pairing is a one-time action; devices remember each other afterwards.

**Prerequisite**: both devices on the **same WiFi / LAN**, both running AA4C.

1. **Discover each other.** Within a few seconds, the other device's name should appear under "Nearby devices" on the home screen. If it does not, jump to [discovery troubleshooting](FAQ.md#devices-are-not-showing-up).
2. **Start pairing.** Click **Pair** on the other device's card.
3. **Compare the confirmation code.** Both screens show a **6-digit number**.

   > ⚠️ **Actually look at both screens and check the digits match.** This is AA4C's one security checkpoint: each device derives those six digits independently and **they never travel over the network**, so a man-in-the-middle cannot make both sides display the same code. If the codes differ, **reject the pairing**.

4. **Confirm on both sides.** Pairing is done.
5. **Is this your own device?** The success dialog asks.
   - Choose **"Yes, this is my device"** → the peer is promoted to **Full trust** and can take part in cross-device indexing and sync.
   - Choose **No** (or dismiss) → it stays at **Friend** level: file exchange and manual shares only, no sync.

   You can change this at any time under *Settings → Paired devices*.

### About trust levels

Pairing is not "here, take all my files". Trust has four levels:

| Level | Who it is for | Can index / sync your files | Receiving your files |
|-------|---------------|------------------------------|----------------------|
| **Full trust** | Your own devices | ✅ Two-way | Auto-accept can be enabled |
| **Friend** | Friends / family / colleagues | ❌ Manual shares only | You confirm each time |
| **Temporary** | A one-off exchange | ❌ | Confirm every time |
| **Unknown** | Discovered, not paired | ❌ | Rejected |

**Pairing lands on Friend by default** — least privilege. Only devices you explicitly promote ever see your sync index.

---

## 3. Interface overview

**Desktop** has a sidebar plus a main area:

| Entry | Purpose |
|-------|---------|
| 🏠 Home | Local status, the five capabilities, nearby devices, recent transfers |
| 📤 Transfer | Pick files → pick a device → AA |
| 🔄 Sync | Manage sync folders, browse files across devices |
| 🔗 Share | Create and manage share links, open someone else's link |
| ⬇️ Download | Download task center |
| 🗂️ Archive | Archive rules, AI suggestions, model library, knowledge base |
| 🕘 History | Transfer history |
| ⚙️ Settings | Device name, directories, download and AI parameters, paired devices |

**Mobile** has five bottom tabs: Home / Transfer / Sync / Download / Me (history, share, archive and settings all live under *Me*).

---

## 4. Transfer: AA a file across

**Transfer page → 1 pick files → 2 pick a device → hit AA.**

### Picking files

- **Drag** files or folders straight into the dashed drop zone
- Or click "Select files" / "Select folder"

Any type, any size. Folders arrive with their directory structure intact.

### Picking a device

Below the drop zone you see paired devices that are currently online. An empty list means the peer is offline, on another network, or not paired yet.

### What the other side sees

- By default, a confirmation dialog appears and the transfer **starts only after they accept**
- If the peer is a **Full trust** device and has "Auto-accept files from trusted devices" enabled in settings, the transfer starts immediately without interrupting them

### During the transfer

- The task bar at the bottom shows live progress and speed; you can cancel at any time
- Every file is verified with **BLAKE3** on arrival, and retransmitted automatically if it does not match
- After a network drop, the transfer **resumes from where it stopped** rather than restarting
- Received files land in the directory configured under *Settings → Save received files to* (default `~/Downloads/AA4C`)

> Everything is TLS 1.3 encrypted, with no option to turn encryption off.

---

## 5. Sync: keep folders consistent across devices

Sync happens between **Full trust** devices — that is, your own devices. Friend-level devices never see your sync index.

### Adding a sync folder

Sync page → **+ Add sync folder** → choose a directory.

AA4C then:
1. Scans the directory and builds a local index (names, sizes, hashes — **content is not copied**)
2. Watches for changes so additions, edits and deletions update the index automatically
3. Exchanges indexes with online Full trust devices

> The inbox (your receive directory) is **included automatically** — no need to add it. Files received on device A are reachable from device B.

### Understanding the three colors

This is the biggest difference from traditional sync tools: **by default only names and structure are synced, not content**.

| Marker | Meaning | What you can do |
|--------|---------|-----------------|
| 🟢 **Local** | Content is on this machine | Open it |
| 🟡 **Fetchable** | Content is on an online device | **Click to retrieve it** — it turns green when done |
| 🔴 **Offline** | Only an offline device has the content | Wait for that device to come online |

The payoff: a 1 TB media library on your NAS can appear in full on a laptop while costing a few MB of index. Fetch only what you actually need.

### Conflicts

When two devices hold files with the same name but different content, AA4C **does not pick a winner for you**. The entry is marked as having multiple versions, listed side by side and numbered; retrieve them separately and decide yourself.

### Manual refresh

- **Refresh devices**: exchange indexes with online devices again
- Rescan: rebuild the local index (rarely needed — file watching is automatic)

---

## 6. Share: send files to friends, family, colleagues

Sharing targets **specific people**. It is not publishing — AA4C has no community and no public resource listings.

### Creating a share link

Share page → pick a file → pick an expiry → **Create share link**.

| Expiry | Use |
|--------|-----|
| 1 hour | A quick hand-off |
| 1 day | |
| 7 days | Default |
| Never expires | Until you revoke it manually |

> You can only share **files that are inside a sync folder**. If the dropdown is empty, add a sync folder first on the Sync page.

Click **Copy** and send the `aa4c://share/…` link however you like.

### Opening someone else's share

Paste the `aa4c://share/…` link into the input at the bottom of the Share page and click open.

### The boundaries of sharing (important)

- A link is valid for **that one file only** — nothing else in the directory is exposed
- **Revocable at any time**: revoke it under "My shares" and it dies immediately
- An **access log** shows who fetched it and when
- Sharing is **not pairing** and does not make the recipient a trusted device
- Across networks it requires a configured self-hosted server with remote connectivity enabled (see the [self-hosting guide](SELF_HOSTING.md)); on the same LAN it works directly

---

## 7. Download: the unified download center

The Download page handles two kinds of links in one task list:

| Type | Example | Engine |
|------|---------|--------|
| Direct link | `https://…`, `http://…`, `ftp://…` | aria2 |
| BitTorrent / magnet | `magnet:?xt=urn:btih:…` | Transmission |

### Adding tasks

Click **Add download** and paste links. **One per line adds them in bulk** — AA4C works out which lines look like links.

### Managing tasks

- Per task: pause / resume / retry / cancel (cancelling lets you also delete the partial file)
- Bulk: pause all / resume all / clear completed
- A status filter (All / Active / Complete / Error) and a search box sit above the list

### After a download finishes

If *Settings → Archive → Auto-archive after download* is on **and** you have enabled at least one archive rule, the file is classified and moved automatically (see the next section). No rules are enabled by default, so out of the box downloads simply stay in the download directory.

### About the download engines

aria2 and Transmission are GPL software. AA4C **does not copy their code**; it runs them as separate child processes reached only over **loopback (127.0.0.1) with a secret regenerated at every start**. They do not listen on the LAN, so other machines cannot reach them.

If the direct-link engine (aria2) crashes, AA4C first tries to reconnect and then to respawn the process, so downloading does not silently stop working. If the respawn also fails, downloads stay unavailable for the rest of the session — restarting the app recovers.

> ⚠️ **Your responsibility**: AA4C is a download tool. It does not provide, index or recommend any content. Follow the law where you live and only download what you have the right to download.

---

## 8. Archive and AI: auto-organizing, model library, knowledge base

The Archive page has four sections. **The governing principle: rules act, AI only suggests — only deterministic rules may move files automatically; AI output is always a suggestion awaiting your confirmation.**

### 8.1 Archive rules (fully usable without AI)

The rule engine is purely deterministic and needs no model.

**Built-in categories** (11, fixed — tags are where you get freedom):

`model / image / video / audio / document / ebook / archive / installer / code / subtitle / other`

Detection uses an extension table plus magic-byte sniffing as a fallback. When the two disagree, **the file header wins** — so renamed extensions are still classified correctly.

**Creating a rule**: click **+ New rule** and fill in:

| Field | Meaning |
|-------|---------|
| Rule name | For your own reference |
| Matching categories | One or more built-in categories |
| Target directory template | Where to file it, relative to the archive root |
| Tags | Comma-separated, optional |

> **Rules are disabled by default.** That is deliberate — moving files automatically is a risky operation and must be something you explicitly turn on.

**When rules run**: after a download completes (provided *Auto-archive after download* is on), or manually against files you select on the Archive page.

### 8.2 Undo

"Recent actions" lists recent archive operations, each with a **one-click undo** that puts the file back where it was. Automation has to be reversible.

### 8.3 Model library

AA4C recognizes local models by **parsing the GGUF header itself** — reading only the first few dozen KB and **never the tensor data**, so even a 40 GB model is inspected instantly. It extracts:

- Architecture (`general.architecture`)
- Model name and parameter size (e.g. 4B)
- Quantization level (e.g. Q4_K_M, Q8_0)
- Context length

The model directory is set under *Settings → AI → Model directory*, and defaults to the same place the built-in "model" archive rule targets — so **download a GGUF → it is archived into the model directory → it appears in the model library immediately**.

### 8.4 Enabling AI

AI is **optional**. Without a model configured, archive rules work exactly as before; you simply have no AI suggestions and no knowledge base.

To enable it:

1. Obtain GGUF model files yourself (from Hugging Face or elsewhere — AA4C does not distribute models)
   - A **chat model** for classification/tag suggestions and knowledge base answers
   - An **embedding model** for knowledge base retrieval (not needed if you only want tag suggestions)
2. Put them in the model directory
3. Select the chat model and embedding model under *Settings → AI*

How it runs:

- Models run on a local `llama-server` instance — **no cloud calls**, works offline
- The engine starts **lazily**: only when needed, so idle AA4C costs no RAM
- It **stops when idle**: by default it exits after 10 minutes of inactivity to free memory (configurable)
- It also binds 127.0.0.1 with a random secret and is never exposed

### 8.5 AI tag and category suggestions

Select files and ask the AI for suggestions. Results appear in a pending list where you **accept** or **ignore** each one.

> **AA4C never moves a file because of an AI judgement until you click accept.** The worst outcome of a wrong guess is a suggestion you ignore.

### 8.6 Local knowledge base

Ask questions about your own files.

1. In the Knowledge base section, add a directory as a source — AA4C scans its text and code files and indexes them locally
2. Ask a question and get an answer **with citations back to the source text**, so you can verify it
3. Re-index a source when its contents change; delete sources you no longer want

Everything happens locally: your documents are never uploaded and never touch a cloud service.

---

## 9. History

The History page is the full transfer log, grouped by time: when, with whom, how many files, how large, and whether it succeeded (failures show the reason).

Each entry offers a shortcut:

- Successful receives: **Open containing folder**
- Failed sends: **Send again** (jumps back to the Transfer page to reselect)

---

## 10. Settings reference

### Basics

| Setting | Meaning |
|---------|---------|
| Device name | What others see under "Nearby devices"; defaults to the hostname |
| Save received files to | Where received files go; default `~/Downloads/AA4C`. This directory is automatically part of the sync index |
| Auto-accept files from trusted devices | When on, files from **Full trust** devices arrive without a confirmation dialog. Off by default |

### Remote connectivity

| Setting | Meaning |
|---------|---------|
| Self-hosted server address | Of the form `aa4c://host:port#fingerprint`, pointing at your own `aa4c-server` |
| Enable remote connectivity | **Off by default.** While off, AA4C stays entirely on the LAN and makes no internet connections. The toggle is disabled until an address is filled in |

See the [self-hosting guide](SELF_HOSTING.md) for deployment.

### Download

> ⚠️ Download settings **take effect after restarting the app** — they are written into the config file generated when an engine starts, and are not hot-reloaded.

| Setting | Meaning |
|---------|---------|
| Download directory | Defaults to the system downloads folder. It **must sit outside your receive directory** — inside it, downloads would be indexed and shared with every Full trust device |
| Download speed limit (KB/s) | Empty = unlimited |
| Concurrent downloads | Empty = engine default (5 for aria2) |
| Max connections per file | Segmented downloading, i.e. what other tools call multi-threaded download; 1–16. Empty falls back to a sensible 5 rather than aria2's own default of 1 |
| BT ratio limit | Stop seeding at this ratio; empty = unlimited |
| BT idle seeding timeout (minutes) | Stop seeding after this long without upload activity; empty = unlimited |

### Archive

| Setting | Meaning |
|---------|---------|
| Archive root | Defaults to `AA4C归档` under your documents folder. Must not nest with the receive or download directories |
| Auto-archive after download | Master switch, on by default. But **every rule is individually disabled by default**, so nothing moves until you enable a rule |

### AI

| Setting | Meaning |
|---------|---------|
| Model directory | Defaults to `<archive root>/模型` |
| Chat model / embedding model | Picked from the model directory; without them AI features stay unavailable |
| Release memory after idle (minutes) | Default 10 |

### Paired devices

Lists every paired device, where you can:
- Change the trust level (Friend ↔ Full trust)
- **Unpair**: the peer immediately loses the right to connect to you

---

## 11. Where your data lives

AA4C keeps nothing hidden — your things stay ordinary files:

| Content | Location |
|---------|----------|
| Received files | Your receive directory — plain files, no wrapper, no proprietary container |
| Downloaded files | Your download directory |
| Archived files | Your archive root |
| Metadata (devices, tasks, index, rules, tags) | A local SQLite database, schema documented in [DATABASE_SCHEMA.md](../../DATABASE_SCHEMA.md) |
| Device private key | A local file with mode 0600 — **never in the database, never in logs, never off the device** |

**Uninstalling AA4C takes none of your data with it.** More detail in [Open, Free and Secure](OPEN_AND_SECURE.md).

---

## Still stuck?

- [FAQ and troubleshooting](FAQ.md)
- [GitHub Issues](https://github.com/HuoTaoCN/AA4C/issues)
- For security vulnerabilities, report privately per [SECURITY.md](../../SECURITY.md) — **do not open a public issue**
