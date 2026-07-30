//! AA4C sidecar 引擎共享设施（AI2.1，从 `aa4c-download` 平移）：拉起/终止打包的
//! 引擎二进制 + 孤儿进程防护，不管通信（数据面由各自引擎的客户端负责，如
//! `aa4c-download` 的回环 JSON-RPC、`aa4c-ai` 的回环 HTTP）。
//!
//! `aa4c-download`（aria2/Transmission）与 `aa4c-ai`（llama-server，AI2.2）
//! 都要拉起打包的 sidecar 二进制，若这套设施留在 `aa4c-download` 里，
//! `aa4c-ai` 就得依赖 `aa4c-download`——两个概念上无关的引擎耦合在一起，
//! 遂平移成独立 crate，两边各自依赖它。

mod orphan_guard;
mod spawner;

pub use orphan_guard::{arm_pdeathsig, protect_with_job_object, OrphanPidfile};
pub use spawner::{EngineChild, KillFuture, ProcessSpawner, SidecarSpawner, SpawnFuture};
