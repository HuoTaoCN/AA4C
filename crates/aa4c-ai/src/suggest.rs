//! AI 标签/分类建议批量队列（V0.5 里程碑 AI3，ARCHIVE_DESIGN.md §5）。
//!
//! **V0.5 无视觉**——输入只有文件名/类别/大小/（文本族文件）开头 ≤8KB 内容，
//! 图片/视频只按文件名与元数据建议。这个 crate 不做文件识别/文本读取
//! （那是 `aa4c-core::archive::detect` 的活，`aa4c-ai` 不依赖 `aa4c-store`，
//! 见 crate 分层），调用方（`aa4c-core`）负责把 [`SuggestInput`] 组好传进来。
//!
//! 结果只存内存态（§10 已确认决策表：AI 建议持久化——不落库，重启即清），
//! 单并发批量队列，失败的文件标记失败、不重试不阻塞后续（同 D3 批量操作
//! "单个失败只跳过"的既有先例）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use aa4c_types::{Aa4cError, ArchiveCategory, CoreEvent, Result, Suggestion};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::service::AiService;

/// 单个文件的建议请求输入——不含任何文件系统读取，纯数据（调用方已经读好）。
#[derive(Debug, Clone)]
pub struct SuggestInput {
    pub path: PathBuf,
    pub category: ArchiveCategory,
    pub size: u64,
    /// 文本族文件的开头内容（≤8KB）；图片/视频/二进制等不适合读取内容的类别
    /// 传 `None`，请求里就只有文件名/类别/大小。
    pub text_head: Option<String>,
}

const ALL_CATEGORIES: [ArchiveCategory; 11] = [
    ArchiveCategory::Model,
    ArchiveCategory::Image,
    ArchiveCategory::Video,
    ArchiveCategory::Audio,
    ArchiveCategory::Document,
    ArchiveCategory::Ebook,
    ArchiveCategory::Archive,
    ArchiveCategory::Installer,
    ArchiveCategory::Code,
    ArchiveCategory::Subtitle,
    ArchiveCategory::Other,
];

/// 批量建议队列：持有一个待确认建议的内存列表 + "是否有批量正在跑"的门闩
/// （一次只允许一个批量任务，简化进度事件的语义——不需要区分"哪个批量"）。
pub struct SuggestEngine {
    ai: Arc<AiService>,
    events: broadcast::Sender<CoreEvent>,
    pending: StdMutex<Vec<Suggestion>>,
    running: StdMutex<bool>,
}

impl SuggestEngine {
    pub fn new(ai: Arc<AiService>, events: broadcast::Sender<CoreEvent>) -> Arc<Self> {
        Arc::new(Self {
            ai,
            events,
            pending: StdMutex::new(Vec::new()),
            running: StdMutex::new(false),
        })
    }

    /// 起一个后台任务跑完整批（单并发，逐个调用）。已有批量在跑时拒绝——
    /// 避免两条并发队列写同一个 `pending`/发交织的进度事件，UI 层的语义会
    /// 变得复杂而这个场景本来就不常见（用户等一批做完再发下一批完全够用）。
    pub fn start_batch(self: &Arc<Self>, inputs: Vec<SuggestInput>) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }
        {
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            if *running {
                return Err(Aa4cError::Unavailable(
                    "a suggestion batch is already running".into(),
                ));
            }
            *running = true;
        }
        let this = self.clone();
        tokio::spawn(async move { this.run_batch(inputs).await });
        Ok(())
    }

    async fn run_batch(self: Arc<Self>, inputs: Vec<SuggestInput>) {
        let total = inputs.len() as u32;
        for (i, input) in inputs.into_iter().enumerate() {
            let suggestion = self.suggest_one(input).await;
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(suggestion);
            let _ = self.events.send(CoreEvent::AiSuggestProgress {
                done: i as u32 + 1,
                total,
            });
        }
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }

    async fn suggest_one(&self, input: SuggestInput) -> Suggestion {
        let id = uuid::Uuid::new_v4().to_string();
        let path = input.path.to_string_lossy().into_owned();
        let request = build_request(&input);
        let outcome = match self.ai.chat_completion(request).await {
            Ok(resp) => parse_suggestion(&resp),
            Err(e) => Err(e.to_string()),
        };
        match outcome {
            Ok((category, tags, reason)) => Suggestion {
                id,
                path,
                category,
                tags,
                reason,
                error: None,
            },
            Err(error) => Suggestion {
                id,
                path,
                category: ArchiveCategory::Other,
                tags: Vec::new(),
                reason: String::new(),
                error: Some(error),
            },
        }
    }

    /// 当前全部待确认建议（含失败项）快照，按生成顺序。
    pub fn list(&self) -> Vec<Suggestion> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 从待确认列表摘除一条（采纳/忽略共用这一步——调用方决定摘除后要不要
    /// 用它的内容去写标签/移动文件）。`id` 不存在返回 `None`（比如已经被
    /// 摘过一次，或列表在两次调用之间被清空）。
    pub fn take(&self, id: &str) -> Option<Suggestion> {
        let mut guard = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let idx = guard.iter().position(|s| s.id == id)?;
        Some(guard.remove(idx))
    }
}

/// 构造 `/v1/chat/completions` 请求：低温 0.2（ARCHIVE_DESIGN §5）+ JSON Schema
/// 约束输出（AI2.0 实测确认的请求字段形态，见 ARCHIVE_DESIGN §3.1 第 4 点）。
fn build_request(input: &SuggestInput) -> Value {
    let file_name = input
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.path.to_string_lossy().into_owned());

    let mut user_content = format!(
        "文件名：{file_name}\n检测到的类别：{}\n大小（字节）：{}\n",
        input.category.as_str(),
        input.size
    );
    if let Some(text) = &input.text_head {
        user_content.push_str("文件开头内容：\n");
        user_content.push_str(text);
    }

    let category_enum: Vec<&'static str> = ALL_CATEGORIES.iter().map(|c| c.as_str()).collect();

    json!({
        "messages": [
            {
                "role": "system",
                "content": "你是文件归档助手。根据文件名、类别、大小与开头内容给出更精确的分类与若干标签，只输出符合给定 JSON Schema 的结果，不要输出多余文字。"
            },
            {"role": "user", "content": user_content}
        ],
        "temperature": 0.2,
        "stream": false,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "archive_suggestion",
                "schema": {
                    "type": "object",
                    "properties": {
                        "category": {"type": "string", "enum": category_enum},
                        "tags": {"type": "array", "items": {"type": "string"}},
                        "reason": {"type": "string"}
                    },
                    "required": ["category", "tags", "reason"]
                }
            }
        }
    })
}

/// 从 `/v1/chat/completions` 响应里取出模型生成的 JSON（`choices[0].message.content`
/// 是一个 JSON *字符串*，需要再解析一层），映射成 `(category, tags, reason)`。
/// 任何一步失败都返回可读的错误信息，不 panic——微型模型/量化模型说胡话
/// （schema 合规但内容不像样，或者小概率不合规）是预期内的失败模式。
fn parse_suggestion(
    resp: &Value,
) -> std::result::Result<(ArchiveCategory, Vec<String>, String), String> {
    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("response missing choices[0].message.content string")?;
    let parsed: Value = serde_json::from_str(content)
        .map_err(|e| format!("model content is not valid json: {e}"))?;
    let category_str = parsed["category"]
        .as_str()
        .ok_or("missing or non-string \"category\"")?;
    let category = category_str
        .parse::<ArchiveCategory>()
        .map_err(|e| format!("invalid category {category_str:?}: {e}"))?;
    let tags = parsed["tags"]
        .as_array()
        .ok_or("missing or non-array \"tags\"")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let reason = parsed["reason"].as_str().unwrap_or("").to_string();
    Ok((category, tags, reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{require_llama_server, require_tiny_model};
    use aa4c_engine::ProcessSpawner;
    use std::time::Duration;

    #[test]
    fn build_request_carries_category_size_and_text_head() {
        let input = SuggestInput {
            path: PathBuf::from("/tmp/notes/readme.md"),
            category: ArchiveCategory::Document,
            size: 1234,
            text_head: Some("# hello\nworld".into()),
        };
        let req = build_request(&input);
        let user_msg = req["messages"][1]["content"].as_str().unwrap();
        assert!(user_msg.contains("readme.md"));
        assert!(user_msg.contains("document"));
        assert!(user_msg.contains("1234"));
        assert!(user_msg.contains("# hello"));
        assert_eq!(req["temperature"], 0.2);
        assert_eq!(req["response_format"]["type"], "json_schema");
        let schema_enum = req["response_format"]["json_schema"]["schema"]["properties"]["category"]
            ["enum"]
            .as_array()
            .unwrap();
        assert!(schema_enum.iter().any(|v| v == "document"));
        assert_eq!(schema_enum.len(), 11);
    }

    #[test]
    fn build_request_omits_text_head_when_none() {
        let input = SuggestInput {
            path: PathBuf::from("/tmp/movie.mp4"),
            category: ArchiveCategory::Video,
            size: 42,
            text_head: None,
        };
        let req = build_request(&input);
        let user_msg = req["messages"][1]["content"].as_str().unwrap();
        assert!(!user_msg.contains("文件开头内容"));
    }

    #[test]
    fn parse_suggestion_extracts_fields_from_nested_content_string() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": "{\"category\":\"document\",\"tags\":[\"笔记\",\"markdown\"],\"reason\":\"看起来是笔记\"}"
                }
            }]
        });
        let (category, tags, reason) = parse_suggestion(&resp).unwrap();
        assert_eq!(category, ArchiveCategory::Document);
        assert_eq!(tags, vec!["笔记", "markdown"]);
        assert_eq!(reason, "看起来是笔记");
    }

    #[test]
    fn parse_suggestion_rejects_invalid_category() {
        let resp = json!({
            "choices": [{"message": {"content": "{\"category\":\"nonsense\",\"tags\":[],\"reason\":\"\"}"}}]
        });
        assert!(parse_suggestion(&resp).is_err());
    }

    #[test]
    fn parse_suggestion_rejects_missing_content() {
        let resp = json!({"choices": [{"message": {}}]});
        assert!(parse_suggestion(&resp).is_err());
    }

    /// 引擎不可用（未配置模型）时整批标记失败，不 panic、不阻塞——同
    /// `AiService::chat_completion` 对未配置槽位既有的 `Unavailable` 降级语义。
    #[tokio::test]
    async fn batch_marks_every_item_failed_when_engine_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        let spawner: Arc<dyn aa4c_engine::SidecarSpawner> =
            Arc::new(ProcessSpawner::new("does-not-matter"));
        let (events, mut rx) = broadcast::channel(16);
        let ai = AiService::start(
            spawner,
            crate::service::AiConfig {
                chat_model: None,
                embedding_model: None,
                idle_timeout: Duration::from_secs(600),
                state_dir: dir.path().to_path_buf(),
            },
            events.clone(),
        );
        let engine = SuggestEngine::new(ai, events);

        engine
            .start_batch(vec![SuggestInput {
                path: PathBuf::from("/tmp/a.txt"),
                category: ArchiveCategory::Document,
                size: 10,
                text_head: Some("hi".into()),
            }])
            .unwrap();

        let mut saw_progress = false;
        for _ in 0..50 {
            if let Ok(Ok(CoreEvent::AiSuggestProgress { done, total })) =
                tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
            {
                assert_eq!(done, 1);
                assert_eq!(total, 1);
                saw_progress = true;
                break;
            }
        }
        assert!(saw_progress, "expected an AiSuggestProgress event");

        let pending = engine.list();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].error.is_some());
    }

    /// 两个批量重叠时后一个被拒绝——门闩生效，不产生交织的进度事件。
    #[tokio::test]
    async fn overlapping_batch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spawner: Arc<dyn aa4c_engine::SidecarSpawner> =
            Arc::new(ProcessSpawner::new("does-not-matter"));
        let (events, _rx) = broadcast::channel(16);
        let ai = AiService::start(
            spawner,
            crate::service::AiConfig {
                chat_model: None,
                embedding_model: None,
                idle_timeout: Duration::from_secs(600),
                state_dir: dir.path().to_path_buf(),
            },
            events.clone(),
        );
        let engine = SuggestEngine::new(ai, events);
        let make_input = || SuggestInput {
            path: PathBuf::from("/tmp/a.txt"),
            category: ArchiveCategory::Document,
            size: 1,
            text_head: None,
        };
        engine.start_batch(vec![make_input()]).unwrap();
        let err = engine.start_batch(vec![make_input()]).unwrap_err();
        assert!(matches!(err, Aa4cError::Unavailable(_)));
    }

    /// 真实进程全链路：微型模型对一个真实文本文件出建议，断言 JSON 结构合法
    /// 与流程完整——**不断言建议内容质量**（微型模型说胡话是预期内的，
    /// ARCHIVE_DESIGN.md §5/V0.5_IMPLEMENTATION_PLAN.md AI3 step 4）。
    #[tokio::test]
    async fn real_tiny_model_produces_schema_valid_suggestion() {
        let bin = require_llama_server();
        let model = require_tiny_model();
        let dir = tempfile::tempdir().unwrap();
        let spawner: Arc<dyn aa4c_engine::SidecarSpawner> = Arc::new(ProcessSpawner::new(bin));
        let (events, mut rx) = broadcast::channel(16);
        let ai = AiService::start(
            spawner,
            crate::service::AiConfig {
                chat_model: Some(model),
                embedding_model: None,
                idle_timeout: Duration::from_secs(30),
                state_dir: dir.path().to_path_buf(),
            },
            events.clone(),
        );
        let engine = SuggestEngine::new(ai, events);

        engine
            .start_batch(vec![SuggestInput {
                path: PathBuf::from("/tmp/notes/todo.md"),
                category: ArchiveCategory::Document,
                size: 42,
                text_head: Some("- buy milk\n- write tests".into()),
            }])
            .unwrap();

        let mut done = false;
        for _ in 0..100 {
            if let Ok(Ok(CoreEvent::AiSuggestProgress { done: d, total })) =
                tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
            {
                assert_eq!(d, 1);
                assert_eq!(total, 1);
                done = true;
                break;
            }
        }
        assert!(done, "expected the batch to finish within the timeout");

        let pending = engine.list();
        assert_eq!(pending.len(), 1);
        let suggestion = &pending[0];
        // schema 合法即通过：category 是合法枚举值这件事本身就是 schema 约束
        // 生效的证明（`Suggestion.category` 是强类型 `ArchiveCategory`，
        // 反序列化失败会在 `parse_suggestion` 里变成 `error`，不会跑到这里）。
        if let Some(err) = &suggestion.error {
            panic!("expected a schema-valid suggestion, got error: {err}");
        }
    }
}
