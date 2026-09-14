//! 归档能力（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md）。
//!
//! `detect` 做文件类型识别（扩展名 + magic bytes），`gguf` 解析模型元数据——
//! 两者都是无 I/O 副作用的纯读取，供规则引擎（`engine`）使用。
//!
//! # 为什么是独立 crate（R1）
//!
//! 归档从 AI1 起就住在 `aa4c-core::archive` 里，1700 行——而它的对等物
//! `aa4c-download` 一开始就是独立 crate，两者形态相同却归属不同，纯属历史原因。
//! 归档不是「设备连成一片」的一部分：拿掉它 AA4C 还是 AA4C。所以它和下载一样，
//! 属于插件层（`aa4c_core::plugin`），核心不依赖它。
//!
//! 唯一一处真实耦合是下载完成钩子要读 `archive_auto_enabled` / `archive_root`
//! 两个设置项，此前走 `aa4c_core::settings`。现在本 crate 自己读——这两项本来
//! 就该随归档插件走（R1.3 会把它们正式从 `Settings` 里摘出去），键名与默认值
//! 保持与 `aa4c_core::settings` 逐字一致，语义零变化。

pub mod detect;
pub mod engine;
pub mod gguf;

use std::path::PathBuf;

use aa4c_store::Store;
use aa4c_types::CoreEvent;
use tokio::sync::broadcast;

/// 设置键名，与 `aa4c_core::settings` 中的常量逐字一致（同一张 `settings` 表）。
const KEY_ARCHIVE_ROOT: &str = "archive_root";
const KEY_ARCHIVE_AUTO_ENABLED: &str = "archive_auto_enabled";

/// 平台默认归档根目录：系统文档目录下的 `AA4C归档`（ARCHIVE_DESIGN.md §2.5）。
/// 落在文档目录而不是下载目录，避免和 `download_dir` 产生嵌套歧义。
pub fn default_archive_root() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("AA4C归档")
}

/// 归档钩子需要的两项设置。每次事件重新读，运行期改了立即生效。
struct ArchiveSettings {
    auto_enabled: bool,
    root: PathBuf,
}

/// 读那两个键。`settings` 表里存的是 JSON 标量（同 `aa4c_core::settings::get_json`）。
async fn load_archive_settings(store: &Store) -> aa4c_types::Result<ArchiveSettings> {
    let root = store
        .get_setting(KEY_ARCHIVE_ROOT)
        .await?
        .and_then(|raw| serde_json::from_str::<String>(&raw).ok())
        .map(PathBuf::from)
        .unwrap_or_else(default_archive_root);
    let auto_enabled = store
        .get_setting(KEY_ARCHIVE_AUTO_ENABLED)
        .await?
        .and_then(|raw| serde_json::from_str::<bool>(&raw).ok())
        .unwrap_or(true);
    Ok(ArchiveSettings { auto_enabled, root })
}

/// 下载完成钩子（ARCHIVE_DESIGN.md §2.4）：订阅事件总线，`DownloadDone` 且
/// `archive_auto_enabled` 时跑规则引擎。**只挂这一个事件**——同步/传输收件不自动
/// 归档，收到的文件在 Inbox 索引根内，移走会被同步侧当成删除并向其它设备传播，
/// 属于"无人值守的数据意外"；手动归档不受此限（走 AI1.6 的 Command，不经这个钩子）。
///
/// 每次事件到来都重新读一次设置（不在启动时固定捕获），`archive_auto_enabled`/
/// `archive_root` 运行期改了立即生效，不需要重启应用（同大多数设置项的既有语义）。
pub fn spawn_download_hook(store: Store, events: broadcast::Sender<CoreEvent>) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        loop {
            match sub.recv().await {
                Ok(CoreEvent::DownloadDone { task_id, save_path }) => {
                    let settings = match load_archive_settings(&store).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::debug!(error = %e, "archive hook: settings load failed");
                            continue;
                        }
                    };
                    if !settings.auto_enabled {
                        continue;
                    }
                    let source = std::path::PathBuf::from(&save_path);
                    let archive_root = settings.root;
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

    /// 只写归档插件自己的两个设置键。
    ///
    /// 此前这里要凭空造一个 28 字段的 `aa4c_types::Settings` 再整体 `save`——
    /// 一个只关心 `archive_root`/`archive_auto_enabled` 的测试，却被迫声明
    /// 下载限速、BT 分享率、AI 空闲超时。那正是「设置项全挤在一个结构体里」
    /// 的代价，R1.3 会把它拆掉。
    async fn put_archive_settings(store: &Store, root: &std::path::Path, auto: bool) {
        store
            .set_setting(
                KEY_ARCHIVE_ROOT,
                &serde_json::to_string(&root.to_string_lossy()).unwrap(),
            )
            .await
            .unwrap();
        store
            .set_setting(
                KEY_ARCHIVE_AUTO_ENABLED,
                &serde_json::to_string(&auto).unwrap(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn download_done_triggers_matching_rule_and_updates_save_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let (tx, mut rx) = broadcast::channel::<CoreEvent>(16);

        let archive_root = dir.path().join("archive");
        put_archive_settings(&store, &archive_root, true).await;
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

        spawn_download_hook(store.clone(), tx.clone());

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

        let archive_root = dir.path().join("archive");
        put_archive_settings(&store, &archive_root, false).await;
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

        spawn_download_hook(store.clone(), tx.clone());
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
