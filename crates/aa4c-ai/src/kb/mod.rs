//! 本地知识库（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6/§1）：整体归属这里而不是
//! `aa4c-core`——知识库强依赖嵌入引擎，扫描/分块/嵌入/检索/问答是一条紧耦合的
//! 流水线，拆到两个 crate 只会制造不必要的接缝（§1 分层表）。`aa4c-core` 只做
//! 薄薄一层 Command 转发，不重复这里的逻辑。
//!
//! `aa4c-store` 只依赖 `aa4c-types`，这个 crate 直接依赖 `aa4c-store` 不会形成环。

mod chunk;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use aa4c_store::Store;
use aa4c_types::{
    Aa4cError, CoreEvent, KbAnswerSource, KbDocStatus, KbSource, KbSourceSummary, Result,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::service::AiService;
use chunk::chunk_text;

/// 单文件读取上限（ARCHIVE_DESIGN §6：PDF/Office 不进 V0.5，文本族文件也设硬顶，
/// 防止把一个体积异常的文本文件整个读进内存）。
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// 认为"值得摄入"的文本族扩展名（ARCHIVE_DESIGN §6："md/txt/代码/json/csv 等
/// UTF-8 可读文件"）。宁可漏掉一些冷门后缀，也不把"没有扩展名的文件"这种更
/// 容易踩到二进制文件的情况纳入扫描范围。
const TEXT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "json", "csv", "yaml", "yml", "toml", "rs", "py", "js", "jsx", "ts",
    "tsx", "go", "java", "kt", "c", "h", "cpp", "hpp", "cs", "rb", "sh", "sql", "html", "css",
    "php", "swift", "lua",
];

/// 扫描时跳过的目录名——生成物/依赖目录体量巨大且不是"用户自己的知识"，同隐藏
/// 目录（`.` 开头）一起排除。实用判断，不是穷举。
const SKIP_DIR_NAMES: &[&str] = &["node_modules", "target", ".git", "dist", "build"];

/// 问答检索取 top-k（ARCHIVE_DESIGN §6："query 嵌入 → top-6 chunk"）。
const TOP_K: usize = 6;
/// 单次 `/v1/embeddings` 请求最多带的 chunk 数（个人规模文档一次通常够用；
/// 超出则分批调用，避免单个请求体无节制增长）。
const EMBED_BATCH_SIZE: usize = 32;

/// 知识库服务：来源管理 + 增量摄入 + 暴力余弦检索 + 流式问答。
pub struct KbService {
    ai: Arc<AiService>,
    store: Store,
    events: broadcast::Sender<CoreEvent>,
    /// 一次只允许一个摄入任务在跑（同 `SuggestEngine::running` 的既有门闩语义，
    /// 用普通 `bool` 而不是持有 `MutexGuard` 跨 `tokio::spawn`——后者会有生命周期
    /// 绑在 `&self` 上、挪不进 `'static` 任务的问题）。
    reindexing: StdMutex<bool>,
}

impl KbService {
    pub fn new(
        ai: Arc<AiService>,
        store: Store,
        events: broadcast::Sender<CoreEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            ai,
            store,
            events,
            reindexing: StdMutex::new(false),
        })
    }

    pub async fn add_source(&self, path: PathBuf) -> Result<KbSource> {
        let id = uuid::Uuid::new_v4().to_string();
        self.store
            .insert_kb_source(&id, &path.to_string_lossy())
            .await
    }

    /// 删除来源（级联清空其文档与 chunk，`aa4c-store` 外键负责）。
    pub async fn remove_source(&self, id: &str) -> Result<()> {
        self.store.delete_kb_source(id).await
    }

    pub async fn list_sources(&self) -> Result<Vec<KbSourceSummary>> {
        self.store.list_kb_source_summaries().await
    }

    /// 知识库总 chunk 数（§6"5 万 chunk 起提示知识库偏大"，前端拿这个数字判断
    /// 是否显示警告）。
    pub async fn total_chunks(&self) -> Result<u64> {
        self.store.count_kb_chunks().await
    }

    /// 增量摄入一个来源目录（后台任务，立即返回）：扫描 → 按 mtime+hash 判断
    /// 哪些文件需要（重新）摄入 → 逐个分块+嵌入+落库，单文件失败只跳过（同
    /// D3/AI3"单个失败不阻塞队列"先例）→ 完成后清理"这次扫描没扫到"的已删除
    /// 文件记录。已有摄入在跑时直接拒绝，不排队（同 `SuggestEngine::start_batch`）。
    pub fn reindex(self: &Arc<Self>, source_id: String) -> Result<()> {
        {
            let mut running = self.reindexing.lock().unwrap_or_else(|e| e.into_inner());
            if *running {
                return Err(Aa4cError::Unavailable(
                    "a knowledge base reindex is already running".into(),
                ));
            }
            *running = true;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.run_reindex(&source_id).await;
            *this.reindexing.lock().unwrap_or_else(|e| e.into_inner()) = false;
        });
        Ok(())
    }

    async fn run_reindex(&self, source_id: &str) {
        let source = match self.store.get_kb_source(source_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return, // 来源已被删除，无事可做
            Err(e) => {
                tracing::warn!(error = %e, source_id, "kb: failed to load source for reindex");
                return;
            }
        };

        let files = scan_source_dir(Path::new(&source.path));
        let mut seen_rel_paths = std::collections::HashSet::new();
        let mut pending = Vec::new();
        for (rel_path, abs_path, mtime, size) in files {
            seen_rel_paths.insert(rel_path.clone());
            if size > MAX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&abs_path) else {
                continue;
            };
            let Ok(content) = String::from_utf8(bytes) else {
                continue; // 非 UTF-8，不是这个里程碑要处理的文本族文件
            };
            let hash = blake3::hash(content.as_bytes()).to_hex().to_string();

            let existing = self
                .store
                .get_kb_document_by_rel_path(&source.id, &rel_path)
                .await
                .ok()
                .flatten();
            let needs_embed = match &existing {
                None => true,
                Some(doc) => doc.hash != hash || doc.status != KbDocStatus::Indexed,
            };
            if !needs_embed {
                continue;
            }
            pending.push((rel_path, mtime, hash, content, existing));
        }

        let total = pending.len() as u32;
        for (i, (rel_path, mtime, hash, content, existing)) in pending.into_iter().enumerate() {
            let doc_id = existing
                .map(|d| d.id)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            self.ingest_one(&source.id, &doc_id, &rel_path, mtime, &hash, &content)
                .await;
            let _ = self.events.send(CoreEvent::KbIngestProgress {
                source_id: source.id.clone(),
                done: i as u32 + 1,
                total,
            });
        }

        // 清理已从磁盘消失的文件对应的记录（级联清掉它们的 chunk）。
        if let Ok(existing_docs) = self.store.list_kb_documents(&source.id).await {
            for doc in existing_docs {
                if !seen_rel_paths.contains(&doc.rel_path) {
                    let _ = self.store.delete_kb_document(&doc.id).await;
                }
            }
        }
    }

    /// 单个文件：写入/刷新文档记录（先标 `Pending`，防止半途失败时旧 chunk 被
    /// 误判为对应这份新内容）→ 分块 → 批量嵌入 → 整体替换 chunk → 标 `Indexed`；
    /// 任一步失败标 `Failed`，不中断其余文件（D3/AI3 既有先例）。
    async fn ingest_one(
        &self,
        source_id: &str,
        doc_id: &str,
        rel_path: &str,
        mtime: i64,
        hash: &str,
        content: &str,
    ) {
        if self
            .store
            .upsert_kb_document(
                doc_id,
                source_id,
                rel_path,
                mtime,
                hash,
                KbDocStatus::Pending,
            )
            .await
            .is_err()
        {
            return;
        }

        let chunks = chunk_text(content);
        if chunks.is_empty() {
            // 空文件：没有内容可摄入，直接标完成（不是失败）。
            let _ = self
                .store
                .set_kb_document_status(doc_id, KbDocStatus::Indexed)
                .await;
            let _ = self.store.replace_kb_chunks(doc_id, &[]).await;
            return;
        }

        match self.embed_chunks(&chunks).await {
            Ok(embeddings) => {
                let pairs: Vec<(String, Vec<f32>)> = chunks.into_iter().zip(embeddings).collect();
                if self.store.replace_kb_chunks(doc_id, &pairs).await.is_ok() {
                    let _ = self
                        .store
                        .set_kb_document_status(doc_id, KbDocStatus::Indexed)
                        .await;
                } else {
                    let _ = self
                        .store
                        .set_kb_document_status(doc_id, KbDocStatus::Failed)
                        .await;
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, rel_path, "kb: embedding failed for document");
                let _ = self
                    .store
                    .set_kb_document_status(doc_id, KbDocStatus::Failed)
                    .await;
            }
        }
    }

    /// 分批调用 `/v1/embeddings`（`EMBED_BATCH_SIZE` 一批），按响应的 `index`
    /// 字段归位排序——不假设服务端严格按输入顺序返回（AI2.0 实测过它确实是
    /// 按序的，但 `index` 字段本来就是为这个用途设计的，用它更稳妥）。
    async fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(EMBED_BATCH_SIZE) {
            let resp = self.ai.embeddings(json!({ "input": batch })).await?;
            let data = resp["data"].as_array().ok_or_else(|| {
                Aa4cError::Protocol("embeddings response missing \"data\" array".into())
            })?;
            let mut indexed: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
            for item in data {
                let idx = item["index"].as_u64().unwrap_or(0) as usize;
                let vec = parse_embedding_vec(&item["embedding"])?;
                indexed.push((idx, vec));
            }
            indexed.sort_by_key(|(idx, _)| *idx);
            out.extend(indexed.into_iter().map(|(_, v)| v));
        }
        Ok(out)
    }

    /// 对一个问题起一次流式问答（后台任务，立即返回）：嵌入问题 → 暴力余弦
    /// 检索 top-6 → 拼 prompt → 对话槽位 SSE 流式转发为 `KbAnswerDelta` →
    /// 结束发 `KbAnswerDone{sources}`。引擎不可用/检索失败：不发任何 delta，
    /// 直接发一条带 `error` 的 `KbAnswerDone`（§6：LLM 输出只呈现给人看，
    /// 失败态也只是告知用户，不重试）。
    pub fn ask(self: &Arc<Self>, request_id: String, question: String) {
        let this = self.clone();
        tokio::spawn(async move {
            this.run_ask(request_id, question).await;
        });
    }

    async fn run_ask(&self, request_id: String, question: String) {
        let query_embedding = match self.ai.embeddings(json!({ "input": question })).await {
            Ok(resp) => match parse_embedding_vec(&resp["data"][0]["embedding"]) {
                Ok(v) => v,
                Err(e) => {
                    self.emit_answer_error(&request_id, e.to_string());
                    return;
                }
            },
            Err(e) => {
                self.emit_answer_error(&request_id, e.to_string());
                return;
            }
        };

        let rows = match self.store.list_kb_chunks_for_search().await {
            Ok(rows) => rows,
            Err(e) => {
                self.emit_answer_error(&request_id, e.to_string());
                return;
            }
        };

        let top = top_k_by_cosine(&query_embedding, &rows, TOP_K);
        if top.is_empty() {
            self.emit_answer_error(&request_id, "知识库暂无可用内容".into());
            return;
        }

        let prompt = build_answer_prompt(&question, &top);
        let mut rx = match self
            .ai
            .chat_completion_stream(json!({
                "messages": [
                    {"role": "system", "content": "你是一个只根据给定资料回答问题的助手。资料不足就明确说不知道，不要编造。"},
                    {"role": "user", "content": prompt}
                ],
                "stream": true,
                "temperature": 0.2,
                // 防止跑题模型/没找到停止条件时无限生成——问答场景一个合理上限，
                // 不是流式本身需要的参数（真机验证过 llama-server 认这个字段名，
                // 同 AI2 既有的 `chat_completion_stream` 测试用法一致）。
                "max_tokens": 512,
            }))
            .await
        {
            Ok(rx) => rx,
            Err(e) => {
                self.emit_answer_error(&request_id, e.to_string());
                return;
            }
        };

        while let Some(item) = rx.recv().await {
            let Ok(chunk) = item else { break };
            let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() else {
                continue;
            };
            if delta.is_empty() {
                continue;
            }
            let _ = self.events.send(CoreEvent::KbAnswerDelta {
                request_id: request_id.clone(),
                delta: delta.to_string(),
            });
        }

        let mut seen_paths = std::collections::HashSet::new();
        let sources: Vec<KbAnswerSource> = top
            .into_iter()
            .filter_map(|(row, _)| {
                let path = Path::new(&row.source_path)
                    .join(&row.rel_path)
                    .to_string_lossy()
                    .into_owned();
                seen_paths
                    .insert(path.clone())
                    .then_some(KbAnswerSource { path })
            })
            .collect();
        let _ = self.events.send(CoreEvent::KbAnswerDone {
            request_id,
            sources,
            error: None,
        });
    }

    fn emit_answer_error(&self, request_id: &str, error: String) {
        let _ = self.events.send(CoreEvent::KbAnswerDone {
            request_id: request_id.to_string(),
            sources: Vec::new(),
            error: Some(error),
        });
    }
}

fn parse_embedding_vec(v: &Value) -> Result<Vec<f32>> {
    v.as_array()
        .ok_or_else(|| Aa4cError::Protocol("embedding field is not an array".into()))?
        .iter()
        .map(|n| {
            n.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| Aa4cError::Protocol("embedding element is not a number".into()))
        })
        .collect()
}

/// 递归扫描来源目录，返回 `(相对路径, 绝对路径, mtime 秒, 字节数)`——只收
/// `TEXT_EXTENSIONS` 命中的普通文件，跳过隐藏目录/`SKIP_DIR_NAMES`。目录不存在
/// 或读取失败时返回空列表（不是 panic——来源目录可能在添加之后被移动/删除，
/// 同 `apply_rules` 对"文件不存在"的既有容错姿态）。
fn scan_source_dir(root: &Path) -> Vec<(String, PathBuf, i64, u64)> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf, i64, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            if name.starts_with('.') || SKIP_DIR_NAMES.contains(&name.as_ref()) {
                continue;
            }
            walk(root, &path, out);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let ext_matches = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
        if !ext_matches {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let Ok(rel_path) = path.strip_prefix(root) else {
            continue;
        };
        out.push((
            rel_path.to_string_lossy().into_owned(),
            path.clone(),
            mtime,
            meta.len(),
        ));
    }
}

/// 暴力余弦相似度检索：维度不匹配的行直接跳过（换过嵌入模型后旧 chunk 维度
/// 可能与新 query 不一致，安全跳过比 panic 或算出无意义的数字更好）。
fn top_k_by_cosine<'a>(
    query: &[f32],
    rows: &'a [aa4c_store::KbChunkRow],
    k: usize,
) -> Vec<(&'a aa4c_store::KbChunkRow, f32)> {
    let mut scored: Vec<(&aa4c_store::KbChunkRow, f32)> = rows
        .iter()
        .filter(|r| r.embedding.len() == query.len())
        .map(|r| (r, cosine_similarity(query, &r.embedding)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(k);
    scored
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn build_answer_prompt(question: &str, top: &[(&aa4c_store::KbChunkRow, f32)]) -> String {
    let mut prompt = String::from("仅根据以下资料回答问题，资料不足就说不知道：\n\n");
    for (row, _) in top {
        prompt.push_str("---\n来源：");
        prompt.push_str(&row.rel_path);
        prompt.push('\n');
        prompt.push_str(&row.text);
        prompt.push('\n');
    }
    prompt.push_str("---\n\n问题：");
    prompt.push_str(question);
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{require_llama_server, require_tiny_model};
    use aa4c_engine::ProcessSpawner;
    use std::time::Duration;

    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_is_zero() {
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector_is_zero_not_nan() {
        let sim = cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]);
        assert_eq!(sim, 0.0);
    }

    fn fake_row(rel_path: &str, text: &str, embedding: Vec<f32>) -> aa4c_store::KbChunkRow {
        aa4c_store::KbChunkRow {
            doc_id: "d1".into(),
            source_path: "/notes".into(),
            rel_path: rel_path.into(),
            text: text.into(),
            embedding,
        }
    }

    #[test]
    fn top_k_by_cosine_orders_by_similarity_and_skips_dim_mismatch() {
        let rows = vec![
            fake_row("a.md", "远", vec![0.0, 1.0]),
            fake_row("b.md", "近", vec![1.0, 0.0]),
            fake_row("c.md", "维度不匹配", vec![1.0, 0.0, 0.0]),
        ];
        let top = top_k_by_cosine(&[1.0, 0.0], &rows, 5);
        // c.md 维度不匹配被跳过，只剩 2 条；b.md 更相似排前面
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0.rel_path, "b.md");
        assert_eq!(top[1].0.rel_path, "a.md");
    }

    #[test]
    fn top_k_by_cosine_truncates_to_k() {
        let rows: Vec<_> = (0..10)
            .map(|i| fake_row(&format!("{i}.md"), "x", vec![1.0, i as f32]))
            .collect();
        let top = top_k_by_cosine(&[1.0, 0.0], &rows, 3);
        assert_eq!(top.len(), 3);
    }

    #[test]
    fn scan_source_dir_finds_text_files_skips_binaries_hidden_and_excluded_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "hello").unwrap();
        std::fs::write(dir.path().join("b.bin"), [0u8, 1, 2]).unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("c.txt"), "world").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("d.md"), "should be skipped").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(
            dir.path().join("node_modules").join("e.js"),
            "should be skipped",
        )
        .unwrap();

        let found = scan_source_dir(dir.path());
        let rel_paths: std::collections::HashSet<_> =
            found.iter().map(|(rel, ..)| rel.clone()).collect();
        assert!(rel_paths.contains("a.md"));
        assert!(rel_paths.contains(&format!("sub{}c.txt", std::path::MAIN_SEPARATOR)));
        assert!(!rel_paths.iter().any(|p| p.contains("b.bin")));
        assert!(!rel_paths.iter().any(|p| p.contains(".git")));
        assert!(!rel_paths.iter().any(|p| p.contains("node_modules")));
    }

    /// 真实进程全链路：真实 `llama-server` + 微型模型，摄入一个真实文本文件，
    /// 再问一个能被内容回答的问题，断言：分块+嵌入真的落库、检索真的命中了
    /// 正确的文件、流式回答真的产出了内容、`KbAnswerDone` 带回了这份文件的路径
    /// 作为引用来源。**不断言回答内容质量**——微型模型说胡话是预期内的失败模式
    /// （同 AI3 `real_tiny_model_produces_schema_valid_suggestion` 先例）。
    #[tokio::test]
    async fn reindex_and_ask_real_model_finds_relevant_chunk_and_streams_answer() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let state_dir = tempfile::tempdir().unwrap();
        let notes_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            notes_dir.path().join("todo.md"),
            "买牛奶。\n\n写测试。\n\n给知识库摄入一段可以被检索到的文本内容。",
        )
        .unwrap();

        let spawner: Arc<dyn aa4c_engine::SidecarSpawner> = Arc::new(ProcessSpawner::new(bin));
        let (events, mut rx) = broadcast::channel(256);
        let ai = AiService::start(
            spawner,
            crate::service::AiConfig {
                chat_model: Some(model.clone()),
                embedding_model: Some(model),
                idle_timeout: Duration::from_secs(60),
                state_dir: state_dir.path().to_path_buf(),
            },
            events.clone(),
        );
        let store = Store::open(&state_dir.path().join("kb-test.db"))
            .await
            .unwrap();
        let kb = KbService::new(ai, store, events);

        let source = kb.add_source(notes_dir.path().to_path_buf()).await.unwrap();
        kb.reindex(source.id.clone()).unwrap();

        // 拿一个截止时间而不是固定循环次数：流式回答可能在极短时间内产生远超
        // 循环次数上限的 delta 事件（每条都是"已经排队等着读"，不需要等 500ms
        // 超时），固定次数的 `for _ in 0..N` 会在真正等到 Done 之前就把预算
        // 耗尽——这是本测试最初的一次真实失败，故意换成按墙钟时间预算。
        let ingest_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut ingest_done = false;
        while tokio::time::Instant::now() < ingest_deadline {
            let remaining = ingest_deadline.saturating_duration_since(tokio::time::Instant::now());
            if let Ok(Ok(CoreEvent::KbIngestProgress { done, total, .. })) =
                tokio::time::timeout(remaining, rx.recv()).await
            {
                if done >= total {
                    ingest_done = true;
                    break;
                }
            }
        }
        assert!(ingest_done, "expected ingest to finish within the timeout");
        assert!(kb.total_chunks().await.unwrap() > 0);

        let summaries = kb.list_sources().await.unwrap();
        let summary = summaries.iter().find(|s| s.id == source.id).unwrap();
        assert_eq!(summary.indexed_count, 1);
        assert_eq!(summary.failed_count, 0);

        kb.ask(
            "req-1".into(),
            "知识库里提到了什么可以被检索的内容？".into(),
        );

        let ask_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut got_delta = false;
        let mut done_sources = Vec::new();
        let mut saw_done = false;
        while tokio::time::Instant::now() < ask_deadline {
            let remaining = ask_deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(CoreEvent::KbAnswerDelta { .. })) => got_delta = true,
                Ok(Ok(CoreEvent::KbAnswerDone { sources, error, .. })) => {
                    assert!(error.is_none(), "expected no error, got {error:?}");
                    done_sources = sources;
                    saw_done = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_done, "expected KbAnswerDone within the timeout");
        assert!(got_delta, "expected at least one streamed delta");
        assert!(
            done_sources.iter().any(|s| s.path.ends_with("todo.md")),
            "expected the ingested file to be cited as a source, got {done_sources:?}"
        );
    }
}
