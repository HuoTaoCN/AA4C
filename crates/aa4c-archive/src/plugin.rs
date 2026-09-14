//! 归档 / AI / 知识库的插件外壳（R1，`aa4c_plugin::Plugin`）。
//!
//! 与下载插件不同，这一层**确实搬了逻辑**：`Core` 里那 17 个方法不全是转发，
//! `archive_files` 的三路分支、`list_local_models` 的两层目录扫描、`start_suggest`
//! 的输入组装（类别识别 + 读文本头）都是真代码。它们之所以住在 `aa4c-core`，是因为
//! `aa4c-ai` 不依赖 `aa4c-store`、也不做文件识别（见 ARCHIVE_DESIGN.md §1 的分层）——
//! 那时候没有第三个地方可放。现在有了：本 crate 同时看得见 `aa4c-store`、
//! `aa4c-ai` 和自己的 `detect`/`gguf`/`engine`，正是它们该待的地方。
//!
//! # 三个能力合成一个插件
//!
//! 归档（AI1）、AI 建议（AI3）、知识库（AI4）共用一个 `AiService`（两个模型槽位），
//! 而且「AI 不可用」对三者是同一件事。拆成三个插件只会让那条共享的 `Arc<AiService>`
//! 横跨插件边界，得不偿失。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aa4c_ai::{AiConfig, AiService, KbService, SlotKind, SuggestEngine, SuggestInput};
use aa4c_engine::SidecarSpawner;
use aa4c_plugin::{Plugin, PluginContext, PluginFuture};
use aa4c_store::Store;
use aa4c_types::{Aa4cError, ArchiveCategory, ArchiveRule, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use crate::{default_archive_root, engine};

/// 插件自己的设置项。此前是 `aa4c_types::Settings` 里的 6 个字段。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ArchiveSettings {
    pub archive_root: Option<String>,
    pub auto_enabled: Option<bool>,
    pub models_dir: Option<String>,
    pub chat_model: Option<String>,
    pub embedding_model: Option<String>,
    pub idle_timeout_minutes: Option<u32>,
}

impl ArchiveSettings {
    fn root(&self) -> PathBuf {
        self.archive_root
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_archive_root)
    }

    /// 默认模型目录 `<归档根>/模型`——与内置「模型」归档规则的目标同址
    /// （ARCHIVE_DESIGN.md §3.5：下载 GGUF → 自动归档进模型目录 → 模型库立即可见）。
    fn models_dir(&self) -> PathBuf {
        self.models_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root().join("模型"))
    }
}

/// 运行期状态。`ai` 为 `None` = 本平台/构建没接 AI 引擎（壳层没注入 spawner）；
/// 归档规则引擎**不依赖它**，没有 AI 也完整可用（ARCHIVE_DESIGN.md §2 的核心原则）。
struct Running {
    store: Store,
    settings: ArchiveSettings,
    events: tokio::sync::broadcast::Sender<aa4c_types::CoreEvent>,
    ai: Option<Arc<AiService>>,
    suggest: Option<Arc<SuggestEngine>>,
    kb: Option<Arc<KbService>>,
}

/// 归档插件。由桌面壳层构造（只有它拿得到 AI 引擎的 spawner）。
pub struct ArchivePlugin {
    ai_spawner: Option<Arc<dyn SidecarSpawner>>,
    running: OnceCell<Arc<Running>>,
}

impl ArchivePlugin {
    pub fn new(ai_spawner: Option<Arc<dyn SidecarSpawner>>) -> Self {
        Self {
            ai_spawner,
            running: OnceCell::new(),
        }
    }

    fn running(&self) -> Result<&Arc<Running>> {
        self.running
            .get()
            .ok_or_else(|| Aa4cError::Unavailable("archive plugin is not running".into()))
    }
}

/// AI 三件套共用同一条门闩——没有 AI 引擎，建议和知识库都无从谈起。
/// 文案与 `Core::ai_service()` 此前逐字一致，前端看到的错误不变。
fn unavailable() -> Aa4cError {
    Aa4cError::Unavailable("AI capability not available on this build".into())
}

/// 各方法的入参。按方法各自反序列化，理由同下载插件：`#[serde(untagged)]` 会让
/// 「方法名对、参数形状错」报成「未知方法」。
mod args {
    use super::*;

    #[derive(Debug, Deserialize)]
    pub(super) struct SaveRule {
        pub rule: ArchiveRule,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct Id {
        pub id: String,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct LogId {
        pub log_id: i64,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct ArchiveFiles {
        pub paths: Vec<String>,
        #[serde(default)]
        pub rule_id: Option<String>,
        #[serde(default)]
        pub target_dir: Option<String>,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct Paths {
        pub paths: Vec<String>,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct Resolve {
        pub id: String,
        pub adopt: bool,
        #[serde(default)]
        pub target_dir: Option<String>,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct Path_ {
        pub path: String,
    }
    #[derive(Debug, Deserialize)]
    pub(super) struct Question {
        pub question: String,
    }
}

fn parse<T: serde::de::DeserializeOwned>(method: &str, payload: Value) -> Result<T> {
    serde_json::from_value(payload)
        .map_err(|e| Aa4cError::Protocol(format!("archive.{method}: bad arguments: {e}")))
}

impl Plugin for ArchivePlugin {
    fn id(&self) -> &'static str {
        "archive"
    }

    fn display_name(&self) -> &'static str {
        "归档"
    }

    fn start(&self, ctx: PluginContext) -> PluginFuture<()> {
        let ai_spawner = self.ai_spawner.clone();
        let cell = self.running.clone();
        Box::pin(async move {
            let settings: ArchiveSettings =
                serde_json::from_value(ctx.settings.clone()).unwrap_or_default();

            // 首次启动写入五条**默认停用**的预设规则（装完就悄悄移动用户文件是
            // 意外行为，ARCHIVE_DESIGN.md §2.3）。失败只记 warn，不阻断插件启动。
            if let Err(e) = engine::ensure_default_rules(&ctx.store).await {
                tracing::warn!(error = %e, "ensure default archive rules failed");
            }
            // 下载完成钩子：DownloadDone → 跑规则引擎。只挂这一个事件，理由见
            // `crate::spawn_download_hook` 的文档。
            crate::spawn_download_hook(ctx.store.clone(), ctx.events.clone());

            // AI 引擎懒启动：`AiService::start` 不拉起任何进程，只登记配置。
            let ai = ai_spawner.map(|spawner| {
                AiService::start(
                    spawner,
                    AiConfig {
                        chat_model: settings.chat_model.clone().map(PathBuf::from),
                        embedding_model: settings.embedding_model.clone().map(PathBuf::from),
                        idle_timeout: std::time::Duration::from_secs(
                            u64::from(settings.idle_timeout_minutes.unwrap_or(10)) * 60,
                        ),
                        state_dir: ctx.data_dir.join("ai-state"),
                    },
                    ctx.events.clone(),
                )
            });
            // 建议与知识库借用同一个 `AiService`（对话槽位 / 嵌入槽位），不单独起进程。
            let suggest = ai
                .clone()
                .map(|ai| SuggestEngine::new(ai, ctx.events.clone()));
            let kb = ai
                .clone()
                .map(|ai| KbService::new(ai, ctx.store.clone(), ctx.events.clone()));

            let _ = cell.set(Arc::new(Running {
                store: ctx.store,
                settings,
                events: ctx.events,
                ai,
                suggest,
                kb,
            }));
            Ok(())
        })
    }

    fn shutdown(&self) -> PluginFuture<()> {
        let ai = self.running.get().and_then(|r| r.ai.clone());
        Box::pin(async move {
            if let Some(ai) = ai {
                ai.shutdown().await;
            }
            Ok(())
        })
    }

    fn invoke(&self, method: String, payload: Value) -> PluginFuture<Value> {
        // 借用的生命周期不能跨进 `Box::pin` 的 `async move`，所以先把需要的东西
        // 克隆出来（`Store` / `Arc` 都是廉价克隆，同 Core 此前的做法）。
        let r = self.running().map(|r| {
            (
                r.store.clone(),
                r.settings.clone(),
                r.events.clone(),
                r.ai.clone(),
                r.suggest.clone(),
                r.kb.clone(),
            )
        });
        Box::pin(async move {
            let (store, settings, events, ai, suggest, kb) = r?;
            let m = method.as_str();
            let out = match m {
                // —— 归档规则与条目（AI1）——
                "list_rules" => json!(store.list_archive_rules().await?),
                "save_rule" => {
                    let mut rule = parse::<args::SaveRule>(m, payload)?.rule;
                    // id 为空串 = 新建，由这里生成 uuid（沿用 Core 此前的语义）
                    if rule.id.is_empty() {
                        rule.id = uuid::Uuid::new_v4().to_string();
                    }
                    store.upsert_archive_rule(&rule).await?;
                    let saved = store
                        .list_archive_rules()
                        .await?
                        .into_iter()
                        .find(|r| r.id == rule.id)
                        .ok_or_else(|| {
                            Aa4cError::Protocol("rule not found immediately after upsert".into())
                        })?;
                    json!(saved)
                }
                "delete_rule" => json!(
                    store
                        .delete_archive_rule(&parse::<args::Id>(m, payload)?.id)
                        .await?
                ),
                "list_entries" => json!(store.list_archive_entries().await?),
                "list_log" => json!(store.list_archive_log().await?),
                "undo" => {
                    json!(engine::undo(&store, parse::<args::LogId>(m, payload)?.log_id).await?)
                }
                "archive_files" => {
                    let a: args::ArchiveFiles = parse(m, payload)?;
                    json!(
                        archive_files(
                            &store,
                            &events,
                            &settings.root(),
                            a.paths,
                            a.rule_id,
                            a.target_dir
                        )
                        .await
                    )
                }

                // —— 模型库与 AI 状态（AI2）——
                "list_local_models" => json!(list_local_models(&settings.models_dir())),
                "ai_status" => {
                    let ai = ai.as_ref().ok_or_else(unavailable)?;
                    json!(aa4c_types::AiStatus {
                        chat: ai.status(SlotKind::Chat).await,
                        embedding: ai.status(SlotKind::Embedding).await,
                    })
                }

                // —— AI 标签 / 分类建议（AI3）——
                "start_suggest" => {
                    let engine = suggest.as_ref().ok_or_else(unavailable)?;
                    let a: args::Paths = parse(m, payload)?;
                    json!(engine.start_batch(suggest_inputs(a.paths))?)
                }
                "list_suggestions" => json!(suggest.as_ref().ok_or_else(unavailable)?.list()),
                "resolve_suggestion" => {
                    let engine = suggest.as_ref().ok_or_else(unavailable)?;
                    let a: args::Resolve = parse(m, payload)?;
                    json!(resolve_suggestion(&store, &events, engine, a).await?)
                }

                // —— 本地知识库（AI4）——
                "kb_add_source" => {
                    let kb = kb.as_ref().ok_or_else(unavailable)?;
                    json!(
                        kb.add_source(PathBuf::from(parse::<args::Path_>(m, payload)?.path))
                            .await?
                    )
                }
                "kb_remove_source" => {
                    let kb = kb.as_ref().ok_or_else(unavailable)?;
                    json!(kb.remove_source(&parse::<args::Id>(m, payload)?.id).await?)
                }
                "kb_list_sources" => {
                    json!(kb.as_ref().ok_or_else(unavailable)?.list_sources().await?)
                }
                "kb_reindex" => {
                    let kb = kb.as_ref().ok_or_else(unavailable)?;
                    json!(kb.reindex(parse::<args::Id>(m, payload)?.id)?)
                }
                "kb_ask" => {
                    let kb = kb.as_ref().ok_or_else(unavailable)?;
                    // 返回 request_id 供前端关联后续的 KbAnswerDelta/Done 事件
                    // （同 start_pairing 返回 sessionId 的既有先例）。
                    let request_id = uuid::Uuid::new_v4().to_string();
                    kb.ask(
                        request_id.clone(),
                        parse::<args::Question>(m, payload)?.question,
                    );
                    json!(request_id)
                }

                other => {
                    return Err(Aa4cError::Protocol(format!(
                        "archive: unknown method {other}"
                    )))
                }
            };
            Ok(out)
        })
    }

    fn settings_schema(&self) -> Value {
        json!({
            "title": "归档与 AI",
            "fields": [
                { "key": "archive_root", "type": "dir", "label": "归档根目录",
                  "hint": "必须与接收目录、下载目录互不嵌套" },
                { "key": "auto_enabled", "type": "bool", "label": "下载完成后自动归档",
                  "hint": "总开关。但每条规则自己默认是停用的，不启用规则就不会有任何自动移动" },
                { "key": "models_dir", "type": "dir", "label": "模型文件目录", "hint": "默认 <归档根>/模型" },
                { "key": "chat_model", "type": "file?", "label": "对话模型" },
                { "key": "embedding_model", "type": "file?", "label": "嵌入模型" },
                { "key": "idle_timeout_minutes", "type": "u32", "label": "空闲多久后自动释放内存（分钟）" }
            ]
        })
    }
}

// ———— 以下是从 `aa4c_core::orchestrate` 原样搬来的真实逻辑 ————

/// 批量归档（ARCHIVE_DESIGN §2.4）。`rule_id`：手选某条规则强制应用（不检查匹配
/// 条件）；`target_dir`：完全自定义目标（不经规则、不追加标签）；都不给时退回
/// 自动匹配。**单个文件失败只跳过、记原因，不中断整批**（同 D3 批量操作的既有
/// 取舍），返回实际归档成功的路径。
async fn archive_files(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<aa4c_types::CoreEvent>,
    archive_root: &Path,
    paths: Vec<String>,
    rule_id: Option<String>,
    target_dir: Option<String>,
) -> Vec<String> {
    let mut succeeded = Vec::new();
    for path in paths {
        let source = PathBuf::from(&path);
        let result: Result<Option<PathBuf>> = if let Some(rule_id) = &rule_id {
            engine::apply_selected_rule(store, events, archive_root, &source, rule_id)
                .await
                .map(|(_, to)| Some(to))
        } else if let Some(target_dir) = &target_dir {
            engine::apply_manual(store, events, &source, &PathBuf::from(target_dir))
                .await
                .map(|(_, to)| Some(to))
        } else {
            engine::apply_rules(store, events, archive_root, &source)
                .await
                .map(|outcome| match outcome {
                    engine::ApplyOutcome::Applied { to_path, .. } => Some(to_path),
                    engine::ApplyOutcome::NoRuleMatched => None,
                })
        };
        match result {
            Ok(Some(to_path)) => succeeded.push(to_path.to_string_lossy().into_owned()),
            Ok(None) => tracing::debug!(path = %path, "archive_files: no rule matched, skipped"),
            Err(e) => tracing::warn!(path = %path, error = %e, "archive_files: failed, skipped"),
        }
    }
    succeeded
}

/// 扫描模型目录下的 `.gguf`（递归一层：目录本身 + 各直接子目录，不更深——
/// ARCHIVE_DESIGN.md §3.5）。单个文件解析失败直接跳过；目录不存在返回空列表
/// 而不是错误（首次使用、还没下载任何模型时的正常状态）。
fn list_local_models(root: &Path) -> Vec<aa4c_types::LocalModel> {
    let mut dirs_to_scan = vec![root.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(root) {
        dirs_to_scan.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let mut models = Vec::new();
    for dir in dirs_to_scan {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gguf") {
                continue;
            }
            if let Ok(meta) = crate::gguf::parse_model_meta(&path) {
                models.push(aa4c_types::LocalModel {
                    path: path.to_string_lossy().into_owned(),
                    meta,
                });
            }
        }
    }
    models
}

/// 组装建议引擎的输入：类别识别 + 按需读文本头。
fn suggest_inputs(paths: Vec<String>) -> Vec<SuggestInput> {
    paths
        .into_iter()
        .map(|p| {
            let path = PathBuf::from(p);
            let category = crate::detect::detect_category(&path);
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let text_head = if is_text_like_category(category) {
                read_text_head(&path)
            } else {
                None
            };
            SuggestInput {
                path,
                category,
                size,
                text_head,
            }
        })
        .collect()
}

/// 采纳或忽略一条建议。`id` 已不在待确认列表时返回 `Ok(None)` 而不是报错——
/// UI 重复点击 / 两个窗口都开着归档页这类竞态不该弹错误。
async fn resolve_suggestion(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<aa4c_types::CoreEvent>,
    engine_: &Arc<SuggestEngine>,
    a: args::Resolve,
) -> Result<Option<String>> {
    let Some(suggestion) = engine_.take(&a.id) else {
        return Ok(None);
    };
    if !a.adopt {
        return Ok(None);
    }
    let source = PathBuf::from(&suggestion.path);
    let target_dir = a.target_dir.map(PathBuf::from);
    let (_, to_path) = engine::apply_suggestion(
        store,
        events,
        &source,
        target_dir.as_deref(),
        suggestion.category,
        &suggestion.tags,
    )
    .await?;
    Ok(Some(to_path.to_string_lossy().into_owned()))
}

/// 只有这几类才值得读内容喂给模型——图片/视频/音频/模型/压缩包/安装包都是二进制，
/// 读了也是乱码，白占上下文（ARCHIVE_DESIGN.md §5「V0.5 无视觉」）。
fn is_text_like_category(category: ArchiveCategory) -> bool {
    matches!(
        category,
        ArchiveCategory::Document | ArchiveCategory::Code | ArchiveCategory::Subtitle
    )
}

/// 读文件开头 ≤8KB（ARCHIVE_DESIGN.md §5）。用 `Read::take` 限流而不是整个读进来再
/// 截断——避免「类别偶尔判错、其实是个几 GB 的文件」时的无谓大量 I/O。截断可能落在
/// 多字节字符中间，`from_utf8_lossy` 把尾部替换成 U+FFFD，不影响喂给模型参考。
fn read_text_head(path: &Path) -> Option<String> {
    use std::io::Read;
    const LIMIT: u64 = 8 * 1024;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(LIMIT).read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invoking_before_start_is_unavailable() {
        let plugin = ArchivePlugin::new(None);
        let err = plugin
            .invoke("list_rules".into(), json!({}))
            .await
            .expect_err("should be unavailable");
        assert!(matches!(err, Aa4cError::Unavailable(_)), "got {err:?}");
    }

    /// 模型目录不存在不是错误——首次使用、还没下载任何模型时的正常状态。
    #[test]
    fn a_missing_models_dir_is_empty_not_an_error() {
        assert!(list_local_models(Path::new("/nonexistent/aa4c/models")).is_empty());
    }

    /// 默认模型目录跟着归档根走，而不是另算一个平台路径——内置「模型」归档规则的
    /// 目标目录与它故意同址（下载 GGUF → 自动归档 → 模型库立即可见）。
    #[test]
    fn models_dir_defaults_under_the_archive_root() {
        let s = ArchiveSettings {
            archive_root: Some("/tmp/归档".into()),
            ..Default::default()
        };
        assert_eq!(s.models_dir(), PathBuf::from("/tmp/归档/模型"));
    }

    /// 二进制类别不读内容喂模型（ARCHIVE_DESIGN.md §5「V0.5 无视觉」）。
    #[test]
    fn only_text_like_categories_get_their_head_read() {
        assert!(is_text_like_category(ArchiveCategory::Document));
        assert!(is_text_like_category(ArchiveCategory::Code));
        assert!(!is_text_like_category(ArchiveCategory::Image));
        assert!(!is_text_like_category(ArchiveCategory::Model));
    }

    /// 同下载插件：方法名对、参数错，报的必须是参数问题。
    #[test]
    fn a_known_method_with_bad_arguments_names_the_method_not_a_missing_one() {
        let err = parse::<args::Id>("delete_rule", json!({ "rule_id": "x" })).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("archive.delete_rule"), "{msg}");
        assert!(!msg.contains("unknown method"), "{msg}");
    }
}
