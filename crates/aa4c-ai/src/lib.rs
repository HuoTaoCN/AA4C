//! AA4C 本地 AI 引擎（V0.5 里程碑 AI2，ARCHIVE_DESIGN.md §3）：`llama-server`
//! sidecar 进程 + 手写 OpenAI 兼容 HTTP 客户端 + 懒启动/空闲自停的双槽位服务。
//!
//! 镜像 `aa4c-download` 的形态（同一批依赖倒置 trait：`SidecarSpawner`/
//! `EngineChild`，现在住在共享的 `aa4c-engine`），但生命周期策略完全相反——
//! 下载引擎轻量常驻，这里绝不常驻（LLM 引擎吃内存）。不依赖 `aa4c-download`
//! （两者都依赖 `aa4c-engine`，这正是 AI2.1 把 sidecar 设施单独抽出来的
//! 原因）。
//!
//! 零网络外呼：只连 `127.0.0.1`（ARCHIVE_DESIGN.md §7）。

#![forbid(unsafe_code)]

mod client;
mod process;
mod service;
mod suggest;
mod util;

pub use client::LlamaClient;
pub use process::{LlamaProcess, SlotKind};
pub use service::{AiConfig, AiService};
pub use suggest::{SuggestEngine, SuggestInput};
