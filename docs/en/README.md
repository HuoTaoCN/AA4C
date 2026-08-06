# AA4C Documentation

> [中文](../README.md) · [Back to project home](../../README.en.md)

## For users

| Document | When to read it |
|----------|-----------------|
| 📖 [User Guide](USER_GUIDE.md) | You want to know how a feature works — transfer, sync, share, download, archive & AI, settings |
| ❓ [FAQ and troubleshooting](FAQ.md) | Something is wrong — devices not found, transfers failing, downloads stuck, AI unavailable |
| 🖥️ [Self-hosting guide](SELF_HOSTING.md) | You want devices on different networks to reach each other |
| 🔓 [Open, Free and Secure](OPEN_AND_SECURE.md) | You want to know where your data goes, how privacy and security are handled, and where the license boundary sits |

**Suggested path for new users**: [README](../../README.en.md) quick start → [User Guide §2](USER_GUIDE.md#2-first-run-pairing-two-devices) to pair your first two devices → [FAQ](FAQ.md) when something breaks.

## For developers

Design and development documents live at the repository root and are written in Chinese:

| Document | Contents |
|----------|----------|
| [PROJECT_VISION.md](../../PROJECT_VISION.md) | Product and technical white paper |
| [ARCHITECTURE.md](../../ARCHITECTURE.md) | Overall architecture and crate layout |
| [API_DESIGN.md](../../API_DESIGN.md) | Rust module interface design |
| [PROTOCOL.md](../../PROTOCOL.md) | AA wire protocol specification |
| [DATABASE_SCHEMA.md](../../DATABASE_SCHEMA.md) | SQLite schema |
| [UI_DESIGN_SPEC.md](../../UI_DESIGN_SPEC.md) | UI and interaction specification |
| [TESTING.md](../../TESTING.md) | Test strategy and acceptance checklist |
| [CONTRIBUTING.md](../../CONTRIBUTING.md) | Contribution guide |
| [SECURITY.md](../../SECURITY.md) | Security policy and threat model |
| [ROADMAP.md](../../ROADMAP.md) | Roadmap |
| [CHANGELOG.md](../../CHANGELOG.md) | Changelog |

### Per-module design

| Document | Capability |
|----------|------------|
| [SYNC_DESIGN.md](../../SYNC_DESIGN.md) | AA Sync: trust levels, cross-device index, on-demand fetch, conflicts |
| [CONNECT_DESIGN.md](../../CONNECT_DESIGN.md) | AA Connect / Share: signaling, relay, NAT traversal, share links |
| [DOWNLOAD_DESIGN.md](../../DOWNLOAD_DESIGN.md) | AA Download: engine integration, task model, limits and compliance |
| [ARCHIVE_DESIGN.md](../../ARCHIVE_DESIGN.md) | AA Archive & AI: rule engine, GGUF parsing, AI suggestions, knowledge base |
| [TOUCH_DESIGN.md](../../TOUCH_DESIGN.md) | AA Touch / Direct: NFC, WiFi Direct, Bluetooth (design draft) |

## A note on languages

User-facing documentation is bilingual. Architecture and protocol documents are currently Chinese only — translation contributions are very welcome, see [CONTRIBUTING.md](../../CONTRIBUTING.md).
