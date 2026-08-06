//! 归档能力（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md）。
//!
//! `detect` 做文件类型识别（扩展名 + magic bytes），`gguf` 解析模型元数据——
//! 两者都是无 I/O 副作用的纯读取，供后面的规则引擎（AI1.4）使用。

pub mod detect;
pub mod engine;
pub mod gguf;

use aa4c_store::Store;
use aa4c_types::CoreEvent;
use tokio::sync::broadcast;

use crate::settings;

/// 下载完成钩子（ARCHIVE_DESIGN.md §2.4）：订阅事件总线，`DownloadDone` 且
/// `archive_auto_enabled` 时跑规则引擎。**只挂这一个事件**——同步/传输收件不自动
/// 归档，收到的文件在 Inbox 索引根内，移走会被同步侧当成删除并向其它设备传播，
/// 属于"无人值守的数据意外"；手动归档不受此限（走 AI1.6 的 Command，不经这个钩子）。
///
/// 每次事件到来都重新读一次设置（不在启动时固定捕获），`archive_auto_enabled`/
/// `archive_root` 运行期改了立即生效，不需要重启应用（同大多数设置项的既有语义）。
pub(crate) fn spawn_download_hook(
    store: Store,
    events: broadcast::Sender<CoreEvent>,
    fallback_name: String,
    fallback_save_dir: String,
) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        loop {
            match sub.recv().await {
                Ok(CoreEvent::DownloadDone { task_id, save_path }) => {
                    let settings =
                        match settings::load(&store, &fallback_name, &fallback_save_dir).await {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::debug!(error = %e, "archive hook: settings load failed");
                                continue;
                            }
                        };
                    if !settings.archive_auto_enabled {
                        continue;
                    }
                    let source = std::path::PathBuf::from(&save_path);
                    let archive_root = std::path::PathBuf::from(&settings.archive_root);
                    match engine::apply_rules(&store, &events, &archive_root, &source).await {
                        Ok(engine::ApplyOutcome::Applied { to_path, .. }) => {
                            if let Err(e) = store
                                .update_download_save_path(&task_id, &to_path.to_string_lossy())
                                .await
                            {
                                tracing::warn!(
                                    task_id = %task_id, error = %e,
                                    "archive hook: failed to update download save_path"
                                );
                            }
                        }
                        Ok(engine::ApplyOutcome::NoRuleMatched) => {}
                        Err(e) => {
                            tracing::debug!(task_id = %task_id, error = %e, "archive hook: apply_rules failed");
                        }
                    }
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_types::{ArchiveAction, ArchiveCategory, ArchiveMatch, ArchiveRule, DownloadKind};

    #[tokio::test]
    async fn download_done_triggers_matching_rule_and_updates_save_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let (tx, mut rx) = broadcast::channel::<CoreEvent>(16);

        let archive_root = dir.path().join("archive");
        let settings = aa4c_types::Settings {
            device_name: "test".into(),
            save_dir: dir.path().join("save").to_string_lossy().into_owned(),
            auto_accept_from_trusted: false,
            listen_port: 0,
            server_url: None,
            enable_remote: false,
            download_dir: dir.path().join("downloads").to_string_lossy().into_owned(),
            download_speed_limit_kbps: None,
            download_concurrency: None,
            download_max_connections_per_file: None,
            download_upload_limit_kbps: None,
            download_user_agent: None,
            download_proxy: None,
            download_proxy_bypass: None,
            bt_trackers: None,
            download_resume_on_start: false,
            bt_ratio_limit: None,
            bt_idle_seeding_limit_minutes: None,
            archive_root: archive_root.to_string_lossy().into_owned(),
            archive_auto_enabled: true,
            ai_models_dir: dir.path().join("models").to_string_lossy().into_owned(),
            ai_chat_model: None,
            ai_embedding_model: None,
            ai_idle_timeout_minutes: 10,
        };
        settings::save(&store, &settings).await.unwrap();
        store
            .upsert_archive_rule(&ArchiveRule {
                id: "r1".into(),
                name: "模型".into(),
                enabled: true,
                position: 0,
                matcher: ArchiveMatch {
                    categories: vec![ArchiveCategory::Model],
                    extensions: None,
                    glob: None,
                    min_size: None,
                    max_size: None,
                },
                action: ArchiveAction {
                    target_template: "模型".into(),
                    tags: vec![],
                },
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();

        let src_dir = dir.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("model.gguf");
        std::fs::write(
            &src,
            b"GGUF\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        )
        .unwrap();
        store
            .insert_download(
                "gid1",
                DownloadKind::Http,
                "https://example.com/model.gguf",
                None,
            )
            .await
            .unwrap();

        spawn_download_hook(store.clone(), tx.clone(), "fallback".into(), "/tmp".into());

        tx.send(CoreEvent::DownloadDone {
            task_id: "gid1".into(),
            save_path: src.to_string_lossy().into_owned(),
        })
        .unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match rx.recv().await.unwrap() {
                    CoreEvent::ArchiveApplied {
                        to_path, rule_id, ..
                    } => return (to_path, rule_id),
                    _ => continue,
                }
            }
        })
        .await
        .expect("archive hook should apply the matching rule in time");
        assert_eq!(
            event.0,
            archive_root
                .join("模型")
                .join("model.gguf")
                .to_string_lossy()
        );
        assert_eq!(event.1.as_deref(), Some("r1"));

        assert!(!src.exists());
        assert!(archive_root.join("模型").join("model.gguf").exists());

        let task = store.get_download("gid1").await.unwrap().unwrap();
        assert_eq!(
            task.save_path.as_deref(),
            Some(
                archive_root
                    .join("模型")
                    .join("model.gguf")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[tokio::test]
    async fn download_done_skipped_when_auto_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let (tx, _rx) = broadcast::channel::<CoreEvent>(16);

        let settings = aa4c_types::Settings {
            device_name: "test".into(),
            save_dir: dir.path().join("save").to_string_lossy().into_owned(),
            auto_accept_from_trusted: false,
            listen_port: 0,
            server_url: None,
            enable_remote: false,
            download_dir: dir.path().join("downloads").to_string_lossy().into_owned(),
            download_speed_limit_kbps: None,
            download_concurrency: None,
            download_max_connections_per_file: None,
            download_upload_limit_kbps: None,
            download_user_agent: None,
            download_proxy: None,
            download_proxy_bypass: None,
            bt_trackers: None,
            download_resume_on_start: false,
            bt_ratio_limit: None,
            bt_idle_seeding_limit_minutes: None,
            archive_root: dir.path().join("archive").to_string_lossy().into_owned(),
            archive_auto_enabled: false,
            ai_models_dir: dir.path().join("models").to_string_lossy().into_owned(),
            ai_chat_model: None,
            ai_embedding_model: None,
            ai_idle_timeout_minutes: 10,
        };
        settings::save(&store, &settings).await.unwrap();
        store
            .upsert_archive_rule(&ArchiveRule {
                id: "r1".into(),
                name: "模型".into(),
                enabled: true,
                position: 0,
                matcher: ArchiveMatch {
                    categories: vec![ArchiveCategory::Model],
                    extensions: None,
                    glob: None,
                    min_size: None,
                    max_size: None,
                },
                action: ArchiveAction {
                    target_template: "模型".into(),
                    tags: vec![],
                },
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();

        let src_dir = dir.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("model.gguf");
        std::fs::write(&src, b"GGUF\x03\x00\x00\x00").unwrap();

        spawn_download_hook(store.clone(), tx.clone(), "fallback".into(), "/tmp".into());
        tx.send(CoreEvent::DownloadDone {
            task_id: "gid1".into(),
            save_path: src.to_string_lossy().into_owned(),
        })
        .unwrap();

        // 关掉总闸时不该动文件——没有事件可等，只能用一小段等待时间确认它确实没发生
        // （既有先例：本项目其它"确认某事没发生"的测试也是短等待，见 HANDOFF.md 的
        // 既有惯例，不是新发明的模式）。
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(src.exists(), "auto archive disabled, file must stay put");
        assert_eq!(store.list_archive_log().await.unwrap().len(), 0);
    }
}
