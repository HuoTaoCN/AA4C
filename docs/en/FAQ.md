# FAQ and Troubleshooting

> [中文](../FAQ.md) · [User Guide](USER_GUIDE.md) · [Project home](../../README.en.md)

## Contents

- [About the product](#about-the-product)
- [Devices are not showing up](#devices-are-not-showing-up)
- [Pairing](#pairing)
- [Transfers](#transfers)
- [Sync](#sync)
- [Sharing and remote connectivity](#sharing-and-remote-connectivity)
- [Downloads](#downloads)
- [AI and knowledge base](#ai-and-knowledge-base)
- [Privacy and security](#privacy-and-security)
- [Installation and platforms](#installation-and-platforms)

---

## About the product

### How is this different from LocalSend / Syncthing / Motrix?

Each of those solves one problem. AA4C puts several of them behind **one device identity in one app**: a file you receive can go straight into sync scope, a finished download can be auto-archived and then synced to your NAS — no tool switching, no re-pairing.

Some concrete differences:

- Sync **transfers names, not content, by default** (content is fetched on click), so a small-disk device can still "own" a large library
- Trust is graded after pairing — **pairing is not handing over all your files**
- Signaling and relay for cross-network use are **self-hosted only**; there are no official nodes

### Do I need an account? Does it need internet?

No account. **Remote connectivity is off by default**, and while it is off AA4C only operates on your LAN and makes no internet connections. No telemetry, no crash reporting, no "anonymous statistics".

### Is it free? Will it become paid?

Apache-2.0, fully open source. You can always build it from source yourself.

### Do my files pass through your servers?

No — AA4C **has no servers**. On a LAN, devices connect directly. Across networks, if hole punching fails and a relay is needed, that relay is a server **you deployed yourself**, and it only ever sees ciphertext and endpoint mappings, never file contents.

### Is there an iOS version?

Not yet. Android has an experimental APK; iOS / iPad are planned ([ROADMAP.md](../../ROADMAP.md)).

### Is there a NAS or Docker version?

`aa4c-server` (signaling + relay) ships as a Linux x86_64 binary you can run on a NAS or VPS — see the [self-hosting guide](SELF_HOSTING.md). A full NAS build with UI is a V1.0 goal.

---

## Devices are not showing up

The most common problem. Work through these in order.

### 1. Are both devices on the same network?

They must be on the **same LAN / same WiFi**. Common traps:

- Some routers expose **2.4 GHz and 5 GHz as two isolated networks**
- **Guest WiFi** is usually isolated from the main network
- A **VPN or proxy** on your phone can hijack LAN traffic — turn it off and retry
- One machine is on Ethernet and the other on WiFi, but they are on different subnets

### 2. Is the firewall allowing it?

AA4C uses **TCP 42420** (transfer) and **UDP 5353** (mDNS discovery):

| OS | What to do |
|----|------------|
| Windows | The first-run firewall prompt must have **Private networks** ticked. If you dismissed it, go to "Windows Defender Firewall → Allow an app through firewall", find AA4C, and tick Private |
| macOS | Click **Allow** when asked to accept incoming connections. You can verify later under System Settings → Network → Firewall → Options |
| Linux (ufw) | `sudo ufw allow 42420/tcp && sudo ufw allow 5353/udp` |

### 3. Does the network block multicast?

Discovery relies on mDNS multicast. These environments frequently break it, and **AA4C cannot work around them**:

- Corporate and campus networks (client isolation / AP isolation is common)
- Public WiFi in cafés and airports
- Some ISP-supplied routers in their default configuration

**Quick test**: create a hotspot on your phone and connect both devices to it. If discovery works there, the original network is blocking multicast.

### 4. Discovery fails on Android

Android needs a `MulticastLock`, which AA4C acquires at startup. If it still fails:

- Confirm the app has local network / location-related permissions (some ROMs tie WiFi scanning to location)
- Some vendor ROMs freeze background apps aggressively — add AA4C to the "unrestricted" / "allow background" list

### 5. Still nothing

Restart AA4C on both ends to force a fresh announcement. If that does not help, open an [issue](https://github.com/HuoTaoCN/AA4C/issues) with your OS, network setup and firewall state.

---

## Pairing

### The two devices show different 6-digit codes

**Do not confirm. Reject.**

Each device derives those digits **independently**, and they never cross the network, so under normal conditions they always match. A mismatch means one of two things:

1. You are not looking at the same pairing attempt (e.g. several were started at once) — cancel everything and retry
2. Someone is attacking this pairing — move to a network you trust and try again

### We paired, but the other device does not see my synced files

Pairing lands on **Friend** by default, and friends do not take part in cross-device indexing. Go to *Settings → Paired devices* and promote the peer to **Full trust** — assuming it really is your own device.

### How do I unpair?

*Settings → Paired devices → Unpair.* The peer immediately loses the right to connect to you.

---

## Transfers

### The transfer was interrupted — do I start over?

No. AA4C **resumes from where it stopped** once the connection is back.

### It says verification failed

Every file is BLAKE3-verified on arrival, and mismatches trigger an automatic retransmission. Repeated failures usually mean an unstable link (weak WiFi, a bad cable) — try a different network.

### The other side never got the confirmation dialog

- Their app may be suspended in the background (especially on phones) — ask them to bring it to the foreground
- If they enabled "Auto-accept files from trusted devices", there is no dialog by design

### Where did the received files go?

*Settings → Save received files to*, default `~/Downloads/AA4C`.

### How large a file can I send?

There is no artificial limit. Folders arrive with their structure intact.

---

## Sync

### What does 🟡 "Fetchable" mean?

It means **the name is in the index but the content is not on this machine yet**. That is by design: AA4C syncs names and structure by default and fetches content on demand. Click the file to retrieve it — it turns 🟢 when done.

### What about 🔴 "Offline"?

Only a currently-offline device holds the content. Wait for it to come online.

### I added a sync folder but the other device sees nothing

Check three things:

1. Is the peer at **Full trust**? Friend level does not participate in indexing
2. Are both devices online and able to discover each other?
3. Click "Refresh devices" to exchange indexes again

### What are "multiple versions"?

Two devices hold files with the same name but different content. AA4C **will not silently overwrite one with the other** — versions are listed side by side and numbered, so you can retrieve each and decide.

### If I delete a local file, is it deleted everywhere?

Sync propagates index state. Before putting important files into sync scope, try the behaviour you expect on a small test set — and important data should always have a separate backup regardless.

---

## Sharing and remote connectivity

### The recipient cannot open my share link

- **Same LAN**: it should just work; check both sides are online
- **Across networks**: both sides need a **self-hosted server address** configured and **remote connectivity enabled** in settings. AA4C has no official public server — see the [self-hosting guide](SELF_HOSTING.md)

### The file dropdown is empty when creating a share

You can only share **files inside a sync folder**. Add one on the Sync page first.

### Can I take a share back?

Yes. Revoke it under "My shares" and it dies immediately. You can also set a 1 hour / 1 day / 7 day expiry when creating it.

### Does sharing make the recipient a trusted device?

No. Sharing and pairing are independent, and a share covers **that one file** only.

### Why is there no official relay server?

A deliberate product decision. An official server means a single point of failure, running costs, and the risk of one day being compelled to hand over data. AA4C puts the infrastructure **entirely in the user's hands** instead.

---

## Downloads

### Which links are supported?

HTTP / HTTPS / FTP direct links, and BitTorrent / magnet links (`magnet:?xt=urn:btih:…`). Paste **one per line** to add several at once.

### A magnet link sits at 0 and never connects

Magnet downloads depend on DHT and trackers. Slowness or a total stall usually means the content has no active seeders, or the network restricts BitTorrent traffic. Try:

- A better-seeded source
- Checking whether your network (campus, corporate) blocks BitTorrent

### I changed the speed limit / concurrency and nothing happened

Download settings **take effect after restarting the app** — they are written into the config file generated when the engine starts, and are not hot-reloaded.

### Downloads are slower than in other download managers

Raise *Settings → Download → Max connections per file* (segmented downloading, 1–16). Note that many servers cap connections and speed per IP regardless.

### Why wasn't my finished download archived automatically?

**Two conditions** must both hold:

1. *Settings → Archive → Auto-archive after download* is on (it is by default)
2. **At least one archive rule is enabled** (all rules are disabled by default)

This is deliberately conservative — moving files automatically requires your explicit go-ahead.

### What if a download engine crashes?

If the direct-link engine (aria2) crashes, AA4C reconnects and, if needed, respawns the process — usually nothing for you to do. If the respawn also fails, downloads stay unavailable for the rest of the session; **restart the app** to recover.

### Responsibility for downloaded content

AA4C is a download tool. It does not provide, index or recommend any content. Follow the law where you live and only download what you have the right to download.

---

## AI and knowledge base

### AI features show as unavailable

You need to supply GGUF model files yourself:

1. Download a GGUF model (from Hugging Face or elsewhere — AA4C does not distribute models)
2. Put it in *Settings → AI → Model directory*
3. Select a **chat model** in settings; knowledge base retrieval also needs an **embedding model**

### Are my files uploaded somewhere for analysis?

**No.** AI runs entirely on a local `llama-server` instance with no cloud calls, and works offline. There is no third-party AI endpoint anywhere in the source.

### Will the AI move my files on its own?

**No.** This is a hard design boundary: only deterministic rules may move files; **AI output always lands in a pending-confirmation queue** and takes effect only when you accept it. The worst outcome of a wrong guess is a suggestion you ignore.

### AI uses a lot of memory

The engine is **lazy-start and idle-stop**: it launches only when needed and exits after 10 minutes of inactivity by default (adjustable under *Settings → AI*). You can also pick a smaller model, such as a 4B at Q4 quantization.

### Archiving moved a file to the wrong place

Every entry under *Archive → Recent actions* has a **one-click undo** that restores the original location.

### Knowledge base answers are poor

- Check that a suitable embedding model is configured
- A larger chat model generally answers better
- Answers carry **citations** — open them to verify against the source

---

## Privacy and security

### What data does AA4C collect?

**None.** No telemetry, no crash reporting, no anonymous statistics, no accounts. There is no analytics SDK or third-party endpoint in the source — something you can verify in the repository yourself.

### Are transfers encrypted? Can encryption be turned off?

Everything is TLS 1.3 with certificate pinning, and **there is no off switch**. Project rules forbid adding any option that disables encryption or verification.

### Where is my private key?

In a local file with mode 0600. It is **never written to the database, never written to logs, and never leaves the device**.

### Can someone on my LAN reach the bundled engines?

No. aria2 / Transmission / llama-server all bind `127.0.0.1` (loopback) and authenticate with a **secret regenerated at every start**, so other machines on the network cannot reach them.

### Is my data still there after uninstalling?

Yes. Received, downloaded and archived files are plain files in directories you chose — no proprietary format, no encrypted wrapper.

### How do I report a security vulnerability?

**Please do not open a public issue.** Report privately per [SECURITY.md](../../SECURITY.md) (GitHub Security Advisory or security@aa4c.com).

---

## Installation and platforms

### macOS says the developer cannot be verified

The build is **not signed by Apple** (signing requires a paid developer account). Workaround: right-click the app icon → Open → Open. Or allow it under System Settings → Privacy & Security.

If that bothers you, build from source — the scripts are all in the repository.

### Windows says "Windows protected your PC"

Same cause: no code signing. Click "More info" → "Run anyway".

### The AppImage will not run on Linux

Run `chmod +x AA4C*.AppImage` first. If it complains about FUSE, install `libfuse2`, or use the `.deb` / `.rpm` package instead.

### What works in the Android build?

It is experimental and covers mainly **transfer** and **sync**. The download center and AI archiving are desktop-only for now.

### Which OS versions are supported?

The desktop app is built on Tauri 2, so its requirements follow [Tauri 2's supported platforms](https://tauri.app/start/prerequisites/) (Windows 10/11, recent macOS releases, mainstream Linux distributions). Android is experimental.

If you are on something older, installing it is the most reliable test — if it will not install or start, please open an [issue](https://github.com/HuoTaoCN/AA4C/issues) with your OS version.

---

## Did not find your answer?

- Full usage details in the [User Guide](USER_GUIDE.md)
- Questions and feedback: [GitHub Issues](https://github.com/HuoTaoCN/AA4C/issues)
- Design discussion: [GitHub Discussions](https://github.com/HuoTaoCN/AA4C/discussions)
