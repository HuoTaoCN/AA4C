//! 归档 / AI / 知识库插件的端到端测试（R1 之前住在 `aa4c-core/tests/core.rs`）。
//!
//! 与下载插件的 e2e 同构：**驱动真实 `Core`**，能力走 `plugin_invoke`。
//! 两条都要真实 `llama-server` + 微型 GGUF（`AA4C_TEST_LLAMA_SERVER_BIN` /
//! `AA4C_TEST_TINY_GGUF`，见 HANDOFF.md）——没设环境变量时是**显式 panic**，
//! 看起来和真失败一模一样，那不是回归。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aa4c_archive::ArchivePlugin;
use aa4c_core::{Core, CoreConfig};
use aa4c_engine::ProcessSpawner;
use aa4c_types::CoreEvent;
use serde_json::json;
use tokio::time::timeout;

fn from_json<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).expect("plugin returned an unexpected shape")
}

/// 在 PATH 上找一个可执行文件。
fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// 与 `aa4c_ai::util::require_llama_server` 同源：先看环境变量，再回落 PATH。
///
/// **漏掉 PATH 兜底这一半曾让 CI 的 macOS 腿红了一周**（CHANGELOG 0.7.0-preview.1：
/// `brew install llama.cpp` 装到 PATH 上却不设环境变量），所以两条都留着。
fn require_llama_server_bin() -> PathBuf {
    if let Ok(p) = std::env::var("AA4C_TEST_LLAMA_SERVER_BIN") {
        return PathBuf::from(p);
    }
    which_on_path("llama-server")
        .expect("set AA4C_TEST_LLAMA_SERVER_BIN or put llama-server on PATH (see HANDOFF.md)")
}

/// 微型 GGUF 固件（`engines/test-fixtures` release，见 HANDOFF.md）。
fn require_tiny_gguf() -> PathBuf {
    match std::env::var("AA4C_TEST_TINY_GGUF") {
        Ok(p) => PathBuf::from(p),
        Err(_) => panic!("AA4C_TEST_TINY_GGUF not set — see ARCHIVE_DESIGN.md §3.1 第 6 点"),
    }
}

/// AI 建议全链路（V0.5 里程碑 AI3，ARCHIVE_DESIGN.md §5）：真实 `llama-server` + 微型
/// GGUF（同 AI2.0/AI3.1 实证结论，不 mock）走 `Core` 公开方法——`start_suggest`
/// 正确组好输入（文件识别 + 读文本头，这两步是 aa4c-core 的职责，aa4c-ai 不碰
/// 文件系统）、`AiSuggestProgress` 事件如实广播、`resolve_suggestion(adopt=true)`
/// 采纳后文件真的被移动、打上 `TagSource::Ai` 标签、留下可撤销的 `archive_log`
/// 记录并广播 `ArchiveApplied`。需要 `AA4C_TEST_LLAMA_SERVER_BIN`/`AA4C_TEST_TINY_GGUF`
/// 两个环境变量（同 aa4c-ai 的 `require_llama_server`/`require_tiny_model`，见
/// HANDOFF.md 环境要求）——没设就显式 panic，不静默跳过。
#[tokio::test]
async fn ai_suggest_lifecycle_through_core_orchestration() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    // mDNS 对本套件只是噪声：全部用例都用显式地址建连（见文件头注释），而每个
    // `ServiceDaemon` 自带一条 OS 线程 + 一组 5353 组播 socket，并行跑十几个用例时
    // 几十个守护进程互相挤，配对会开始超时。
    config.disable_discovery = true;
    // 端口映射的真实实现会去改**跑测试这台机器所在路由器**的配置。有几个用例会打开
    // `enable_remote`（映射的外层闸），只是碰巧在退避周期内就跑完了才没真发出去——
    // 不能靠这种巧合，直接从结构上关掉。
    config.disable_port_mapping = true;
    config
        .plugins
        .register(Arc::new(ArchivePlugin::new(Some(Arc::new(
            ProcessSpawner::new(require_llama_server_bin()),
        )))));
    let core = Core::start(config).await.expect("core starts");

    let mut settings = core.get_settings().await.unwrap();
    settings.ai_chat_model = Some(require_tiny_gguf().to_string_lossy().into_owned());
    core.update_settings(settings).await.unwrap();

    let src = dir.path().join("todo.md");
    tokio::fs::write(&src, "- buy milk\n- write tests\n")
        .await
        .unwrap();

    let mut ev = core.subscribe();
    core.plugin_invoke(
        "archive",
        "start_suggest".into(),
        json!({ "paths": [src.to_string_lossy()] }),
    )
    .await
    .unwrap();

    let mut saw_done = false;
    for _ in 0..200 {
        if let Ok(Ok(CoreEvent::AiSuggestProgress { done, total })) =
            timeout(Duration::from_millis(500), ev.recv()).await
        {
            assert_eq!(total, 1);
            if done >= total {
                saw_done = true;
                break;
            }
        }
    }
    assert!(saw_done, "expected the batch to finish within the timeout");

    let suggestions = from_json::<Vec<aa4c_types::Suggestion>>(
        core.plugin_invoke("archive", "list_suggestions".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(suggestions.len(), 1);
    let suggestion = &suggestions[0];
    if let Some(err) = &suggestion.error {
        panic!("expected a schema-valid suggestion, got error: {err}");
    }
    assert_eq!(suggestion.path, src.to_string_lossy());

    let target_dir = dir.path().join("suggest-target");
    let moved = from_json::<Option<String>>(
        core.plugin_invoke(
            "archive",
            "resolve_suggestion".into(),
            json!({
                "id": suggestion.id,
                "adopt": true,
                "target_dir": target_dir.to_string_lossy(),
            }),
        )
        .await
        .unwrap(),
    )
    .expect("adopting should return the file's final path");
    let to_path = PathBuf::from(&moved);
    assert!(!src.exists(), "file should have been moved on adopt");
    assert!(to_path.exists());
    assert!(to_path.starts_with(&target_dir));

    // 待确认列表应该已经摘掉这一条。
    assert!(from_json::<Vec<aa4c_types::Suggestion>>(
        core.plugin_invoke("archive", "list_suggestions".into(), json!({}))
            .await
            .unwrap(),
    )
    .is_empty());

    let entries = from_json::<Vec<aa4c_types::ArchiveEntry>>(
        core.plugin_invoke("archive", "list_entries".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].current_path, to_path.to_string_lossy());

    let log = from_json::<Vec<aa4c_types::ArchiveLogEntry>>(
        core.plugin_invoke("archive", "list_log".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].to_path, to_path.to_string_lossy());
    assert!(
        log[0].rule_id.is_none(),
        "AI suggestion adopt has no rule_id"
    );

    core.shutdown().await.unwrap();
}

/// 本地知识库全链路（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）：真实 `llama-server` +
/// 微型 GGUF 走 `Core` 公开方法——`kb_add_source` 登记来源、`kb_reindex` 真的扫描
/// 摄入一个真实文本文件（`CoreEvent::KbIngestProgress` 如实广播）、`kb_ask` 真的
/// 检索到相关内容并流式回答（`KbAnswerDelta`/`KbAnswerDone` 如实广播，引用来源
/// 命中被摄入的那个文件）。需要 `AA4C_TEST_LLAMA_SERVER_BIN`/`AA4C_TEST_TINY_GGUF`
/// 两个环境变量，同上一条 AI3 测试。
#[tokio::test]
async fn kb_lifecycle_through_core_orchestration() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    // mDNS 对本套件只是噪声：全部用例都用显式地址建连（见文件头注释），而每个
    // `ServiceDaemon` 自带一条 OS 线程 + 一组 5353 组播 socket，并行跑十几个用例时
    // 几十个守护进程互相挤，配对会开始超时。
    config.disable_discovery = true;
    // 端口映射的真实实现会去改**跑测试这台机器所在路由器**的配置。有几个用例会打开
    // `enable_remote`（映射的外层闸），只是碰巧在退避周期内就跑完了才没真发出去——
    // 不能靠这种巧合，直接从结构上关掉。
    config.disable_port_mapping = true;
    config
        .plugins
        .register(Arc::new(ArchivePlugin::new(Some(Arc::new(
            ProcessSpawner::new(require_llama_server_bin()),
        )))));
    let core = Core::start(config).await.expect("core starts");

    let model = require_tiny_gguf().to_string_lossy().into_owned();
    let mut settings = core.get_settings().await.unwrap();
    settings.ai_chat_model = Some(model.clone());
    settings.ai_embedding_model = Some(model);
    core.update_settings(settings).await.unwrap();

    let notes_dir = dir.path().join("notes");
    tokio::fs::create_dir_all(&notes_dir).await.unwrap();
    tokio::fs::write(
        notes_dir.join("todo.md"),
        "买牛奶。\n\n写测试。\n\n给知识库摄入一段可以被检索到的文本内容。",
    )
    .await
    .unwrap();

    let mut ev = core.subscribe();
    let source = from_json::<aa4c_types::KbSource>(
        core.plugin_invoke(
            "archive",
            "kb_add_source".into(),
            json!({ "path": notes_dir.to_string_lossy() }),
        )
        .await
        .unwrap(),
    );
    core.plugin_invoke("archive", "kb_reindex".into(), json!({ "id": source.id }))
        .await
        .unwrap();

    let ingest_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut ingest_done = false;
    while tokio::time::Instant::now() < ingest_deadline {
        let remaining = ingest_deadline.saturating_duration_since(tokio::time::Instant::now());
        if let Ok(Ok(CoreEvent::KbIngestProgress { done, total, .. })) =
            timeout(remaining, ev.recv()).await
        {
            if done >= total {
                ingest_done = true;
                break;
            }
        }
    }
    assert!(ingest_done, "expected ingest to finish within the timeout");

    let sources = from_json::<Vec<aa4c_types::KbSourceSummary>>(
        core.plugin_invoke("archive", "kb_list_sources".into(), json!({}))
            .await
            .unwrap(),
    );
    let summary = sources.iter().find(|s| s.id == source.id).unwrap();
    assert_eq!(summary.indexed_count, 1);
    assert_eq!(summary.failed_count, 0);

    let request_id = from_json::<String>(
        core.plugin_invoke(
            "archive",
            "kb_ask".into(),
            json!({ "question": "知识库里提到了什么可以被检索的内容？" }),
        )
        .await
        .unwrap(),
    );

    let ask_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut got_delta = false;
    let mut done_sources = Vec::new();
    let mut saw_done = false;
    while tokio::time::Instant::now() < ask_deadline {
        let remaining = ask_deadline.saturating_duration_since(tokio::time::Instant::now());
        match timeout(remaining, ev.recv()).await {
            Ok(Ok(CoreEvent::KbAnswerDelta {
                request_id: rid, ..
            })) if rid == request_id => {
                got_delta = true;
            }
            Ok(Ok(CoreEvent::KbAnswerDone {
                request_id: rid,
                sources,
                error,
            })) if rid == request_id => {
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

    core.plugin_invoke(
        "archive",
        "kb_remove_source".into(),
        json!({ "id": source.id }),
    )
    .await
    .unwrap();
    assert!(from_json::<Vec<aa4c_types::KbSourceSummary>>(
        core.plugin_invoke("archive", "kb_list_sources".into(), json!({}))
            .await
            .unwrap(),
    )
    .is_empty());

    core.shutdown().await.unwrap();
}

/// 归档全链路（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md）：走 `Core` 公开方法（不是绕过编排层
/// 直接测内部模块），验证 `Core::start` 已经如实装配好了归档能力——预设规则真的写进去了、
/// 手动归档 Command 真的移动了真实文件、`list_archive_log`/`undo_archive` 真的能把它挪回去。
#[tokio::test]
async fn archive_lifecycle_through_core_orchestration() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    config.disable_discovery = true;
    config.disable_port_mapping = true;
    // **不注入 AI 引擎**：规则式归档不依赖 AI，没有模型也要完整可用
    // （ARCHIVE_DESIGN.md §2 的核心原则）。这条测试正是它的守卫。
    config.plugins.register(Arc::new(ArchivePlugin::new(None)));
    let a = Core::start(config).await.expect("core starts");

    // Core::start 应该已经写入 5 条默认停用的预设规则（AI1.5 的 ensure_default_rules）。
    let presets = from_json::<Vec<aa4c_types::ArchiveRule>>(
        a.plugin_invoke("archive", "list_rules".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(presets.len(), 5);
    assert!(presets.iter().all(|r| !r.enabled));

    // 新建一条自定义规则并启用（走 Command 层的 save_archive_rule，id 传空串触发新建）。
    let saved = from_json::<aa4c_types::ArchiveRule>(
        a.plugin_invoke(
            "archive",
            "save_rule".into(),
            json!({ "rule": aa4c_types::ArchiveRule {
            id: String::new(),
            name: "测试文档规则".into(),
            enabled: true,
            position: 99,
            matcher: aa4c_types::ArchiveMatch {
                categories: vec![aa4c_types::ArchiveCategory::Document],
                extensions: None,
                glob: None,
                min_size: None,
                max_size: None,
            },
            action: aa4c_types::ArchiveAction {
                target_template: "文档测试".into(),
                tags: vec!["测试".into()],
            },
            created_at: 0,
            updated_at: 0,
        } }),
        )
        .await
        .unwrap(),
    );
    assert!(
        !saved.id.is_empty(),
        "core should generate a uuid for the new rule"
    );

    // 手动归档（target_dir 覆写，不经规则匹配）：真实文件、真实移动。
    let src = dir.path().join("note.txt");
    tokio::fs::write(&src, b"hello archive").await.unwrap();
    let target_dir = dir.path().join("manual-target");
    let done = from_json::<Vec<String>>(
        a.plugin_invoke(
            "archive",
            "archive_files".into(),
            json!({
                "paths": [src.to_string_lossy()],
                "rule_id": null,
                "target_dir": target_dir.to_string_lossy(),
            }),
        )
        .await
        .unwrap(),
    );
    assert_eq!(done.len(), 1);
    let to_path = std::path::PathBuf::from(&done[0]);
    assert!(!src.exists());
    assert!(to_path.exists());

    let entries = from_json::<Vec<aa4c_types::ArchiveEntry>>(
        a.plugin_invoke("archive", "list_entries".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].current_path, to_path.to_string_lossy());

    // 撤销：从 list_archive_log 拿到 log id，undo 之后文件应该回到原位。
    let log = from_json::<Vec<aa4c_types::ArchiveLogEntry>>(
        a.plugin_invoke("archive", "list_log".into(), json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(log.len(), 1);
    assert!(log[0].rule_id.is_none(), "manual archive has no rule_id");
    a.plugin_invoke("archive", "undo".into(), json!({ "log_id": log[0].id }))
        .await
        .unwrap();
    assert!(src.exists(), "file should be back at its original path");
    assert!(!to_path.exists());

    a.plugin_invoke("archive", "delete_rule".into(), json!({ "id": saved.id }))
        .await
        .unwrap();
    assert_eq!(
        from_json::<Vec<aa4c_types::ArchiveRule>>(
            a.plugin_invoke("archive", "list_rules".into(), json!({}))
                .await
                .unwrap(),
        )
        .len(),
        5
    );

    a.shutdown().await.unwrap();
}
