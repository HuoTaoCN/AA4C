//! `llama-server` 子进程生命周期（ARCHIVE_DESIGN.md §3.2）：拉起、等它就绪、
//! 优雅退出。形状照抄 `TransmissionProcess`（`aa4c-download`）先例，但配置
//! 面完全不同——**全部走环境变量，不传任何命令行参数**（`--help`/真机端到
//! 端跑通已在 AI2.0 实证：`LLAMA_ARG_MODEL`/`LLAMA_ARG_PORT`/`LLAMA_ARG_HOST`/
//! `LLAMA_ARG_CTX_SIZE`/`LLAMA_API_KEY` 足够覆盖非嵌入槽位，嵌入槽位再加
//! `LLAMA_ARG_EMBEDDINGS`/`LLAMA_ARG_POOLING`），没有 Transmission 那种
//! `settings.json` 配置文件要写。

use std::path::{Path, PathBuf};
use std::time::Duration;

use aa4c_engine::{protect_with_job_object, EngineChild, OrphanPidfile, SidecarSpawner};
use aa4c_types::{Aa4cError, Result};

use crate::client::LlamaClient;
use crate::util::{generate_secret, probe_free_port};

/// 健康检查轮询上限（ARCHIVE_DESIGN.md §3.2：模型加载可能要几十秒）。
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(120);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(500);
/// 上下文长度：ARCHIVE_DESIGN.md §3.2 给出的默认值。
const DEFAULT_CTX_SIZE: &str = "8192";

/// 嵌入槽位需要的额外参数（ARCHIVE_DESIGN.md §3.1 第 3 点：不带这两个
/// `/v1/embeddings` 直接 501）——`mean` 是 AI2.0 真机验证过对非专用嵌入
/// 模型也能跑通的池化方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Chat,
    Embedding,
}

/// 一个正在运行的 `llama-server` 子进程 + 连它需要的连接信息。
pub struct LlamaProcess {
    child: Box<dyn EngineChild>,
    pidfile: OrphanPidfile,
    pub port: u16,
    client: LlamaClient,
}

impl LlamaProcess {
    /// 拉起一个新的 `llama-server`：
    /// 1. 先清扫上次崩溃可能遗留的孤儿（PID 文件方案，核对身份匹配才杀）；
    /// 2. 探测空闲端口、生成随机 `LLAMA_API_KEY`；
    /// 3. 用注入的 `spawner` 拉起进程（零命令行参数，全走环境变量）；
    /// 4. 装上孤儿进程防护——Windows Job Object（按 PID 事后归属）；Linux 经
    ///    `ProcessSpawner` 时已经在 spawn 那一步经 `pre_exec` 装好了
    ///    `PR_SET_PDEATHSIG`；macOS 靠这份 PID 文件兜底；
    /// 5. 轮询 `GET /health` 直到就绪或超过 120s——超时会先 `kill` 掉已经
    ///    拉起的子进程再返回错误，不留下一个没人管的僵尸 llama-server。
    ///
    /// `pidfile_path` 由调用方指定（`AiService` 按槽位各给一个不同路径，两个
    /// 槽位各自独立进程、独立 PID 文件，互不干扰）。
    pub async fn spawn(
        spawner: &dyn SidecarSpawner,
        model_path: &Path,
        slot: SlotKind,
        pidfile_path: PathBuf,
    ) -> Result<Self> {
        let pidfile = OrphanPidfile::new(pidfile_path);
        pidfile.sweep();

        let port = probe_free_port()?;
        let api_key = generate_secret();

        let mut envs = vec![
            ("LLAMA_ARG_HOST".to_string(), "127.0.0.1".to_string()),
            ("LLAMA_ARG_PORT".to_string(), port.to_string()),
            (
                "LLAMA_ARG_MODEL".to_string(),
                model_path.to_string_lossy().into_owned(),
            ),
            (
                "LLAMA_ARG_CTX_SIZE".to_string(),
                DEFAULT_CTX_SIZE.to_string(),
            ),
            ("LLAMA_API_KEY".to_string(), api_key.clone()),
        ];
        if slot == SlotKind::Embedding {
            envs.push(("LLAMA_ARG_EMBEDDINGS".to_string(), "1".to_string()));
            envs.push(("LLAMA_ARG_POOLING".to_string(), "mean".to_string()));
        }

        let child = spawner.spawn(&[], &envs).await?;
        let pid = child.pid();
        pidfile.record(pid);
        protect_with_job_object(pid);

        let client = LlamaClient::new(port, &api_key);
        if let Err(e) = wait_until_healthy(&client).await {
            child.kill().await;
            pidfile.clear();
            return Err(e);
        }

        Ok(Self {
            child,
            pidfile,
            port,
            client,
        })
    }

    pub fn client(&self) -> &LlamaClient {
        &self.client
    }

    /// 优雅退出：按 idle-timeout 触发（不是请求中途打断），`EngineChild::kill`
    /// 幂等、尽力而为，同 `TransmissionProcess::shutdown` 的既有先例——
    /// `llama-server` 没有 aria2 那种 RPC `shutdown` 命令可调，直接终止即可。
    pub async fn shutdown(&self) {
        self.child.kill().await;
        self.pidfile.clear();
    }

    pub fn recent_stdio(&self) -> Vec<String> {
        self.child.recent_stdio()
    }
}

async fn wait_until_healthy(client: &LlamaClient) -> Result<()> {
    let deadline = tokio::time::Instant::now() + HEALTH_CHECK_TIMEOUT;
    loop {
        if client.health().await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Aa4cError::Unavailable(
                "llama-server did not become healthy within 120s".into(),
            ));
        }
        tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_engine::ProcessSpawner;
    use std::path::PathBuf;

    /// 同 `aa4c-download` 的 `require_aria2c` 先例：找不到就 panic 报安装
    /// 指引，不静默跳过（V0.4_IMPLEMENTATION_PLAN.md D1 步骤 4 定的规矩，
    /// AI2 沿用）。优先找 PATH（`brew install llama.cpp` 提供，AI2.3 CI 会
    /// 装），`AA4C_TEST_LLAMA_SERVER_BIN` 是本机手动指定 AI2.0 已验证过的
    /// 裁剪产物二进制路径的逃生舱。
    fn require_llama_server() -> PathBuf {
        if let Ok(p) = std::env::var("AA4C_TEST_LLAMA_SERVER_BIN") {
            return PathBuf::from(p);
        }
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let exe_name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(exe_name);
            if candidate.is_file() {
                return candidate;
            }
        }
        panic!(
            "llama-server not found in PATH and AA4C_TEST_LLAMA_SERVER_BIN not set — \
             install it to run aa4c-ai tests (macOS: `brew install llama.cpp`; or point \
             AA4C_TEST_LLAMA_SERVER_BIN at the AI2.0-verified trimmed binary). See \
             ARCHIVE_DESIGN.md §3.1/HANDOFF.md."
        );
    }

    /// `engines/test-fixtures` release 里的 `stories260K.gguf`（ARCHIVE_DESIGN.md
    /// §3.1 第 6 点：来源、大小、SHA256 均已实证）。没有内建自动下载——测试
    /// 环境需要的是"这个文件已经在本机某处"这个事实，下载逻辑属于
    /// `fetch-engines.sh`/CI 的职责，不是测试自己该做的事。
    fn require_tiny_model() -> PathBuf {
        match std::env::var("AA4C_TEST_TINY_GGUF") {
            Ok(p) => PathBuf::from(p),
            Err(_) => panic!(
                "AA4C_TEST_TINY_GGUF not set — point it at a local copy of stories260K.gguf \
                 (download from the project's `engines/test-fixtures` GitHub release, SHA256 \
                 in ARCHIVE_DESIGN.md §3.1 第 6 点) to run this test."
            ),
        }
    }

    /// 真实进程全链路：拉起真正的 `llama-server`（微型模型），等它就绪，
    /// 打一次 `/health`，再优雅退出——不 mock。CI/本机没有装好这两样时
    /// `require_llama_server`/`require_tiny_model` 会直接 panic 报安装指引
    /// （同 `require_aria2c` 的既有先例：集成测试要求真实二进制存在，缺失时
    /// 显式失败，不静默跳过）。
    #[tokio::test]
    async fn spawns_llama_server_and_reaches_healthy_state() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let spawner = ProcessSpawner::new(bin);
        let dir = tempfile::tempdir().unwrap();

        let proc = LlamaProcess::spawn(
            &spawner,
            &model,
            SlotKind::Chat,
            dir.path().join("chat.pid"),
        )
        .await
        .unwrap();
        proc.client().health().await.unwrap();
        proc.shutdown().await;
        assert!(!dir.path().join("chat.pid").exists());
    }

    /// 真实进程 + 真实 SSE 流式响应端到端跑通——`client.rs` 里的单测只喂了
    /// 合成字节验证分帧/解码逻辑本身，这里补上真正连到 `llama-server`、
    /// 真正收多个 chunk、真正看到 `[DONE]` 收尾的那一段（AI2.0 抓包确认过
    /// 的行为，AI2.2 得有一个真实进程测试覆盖到，不能只停在合成数据层）。
    #[tokio::test]
    async fn chat_completion_stream_receives_multiple_real_chunks() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let spawner = ProcessSpawner::new(bin);
        let dir = tempfile::tempdir().unwrap();

        let proc = LlamaProcess::spawn(
            &spawner,
            &model,
            SlotKind::Chat,
            dir.path().join("chat.pid"),
        )
        .await
        .unwrap();

        let mut rx = proc
            .client()
            .chat_completion_stream(serde_json::json!({
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true,
                "max_tokens": 6
            }))
            .unwrap();

        let mut chunk_count = 0;
        while let Some(item) = rx.recv().await {
            let v = item.unwrap();
            assert_eq!(v["object"], "chat.completion.chunk");
            chunk_count += 1;
        }
        assert!(
            chunk_count > 1,
            "expected multiple streamed chunks, got {chunk_count}"
        );

        proc.shutdown().await;
    }

    #[tokio::test]
    async fn embedding_slot_answers_v1_embeddings() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let spawner = ProcessSpawner::new(bin);
        let dir = tempfile::tempdir().unwrap();

        let proc = LlamaProcess::spawn(
            &spawner,
            &model,
            SlotKind::Embedding,
            dir.path().join("embed.pid"),
        )
        .await
        .unwrap();
        let resp = proc
            .client()
            .embeddings(serde_json::json!({ "input": "hello world" }))
            .await
            .unwrap();
        assert!(resp["data"][0]["embedding"].is_array());
        proc.shutdown().await;
    }
}
