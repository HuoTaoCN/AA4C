//! 本地文件索引扫描（SYNC_DESIGN.md §3.2、§10 里程碑 2）。
//!
//! V0.2 里程碑 2 范围：只做「定时扫描 + 手动/事件触发扫描」，按 mtime+size
//! 惰性计算 BLAKE3；文件系统实时监听（`notify` crate）留给后续里程碑——
//! 设计文档已把这点列为已知简化（见 SYNC_DESIGN.md §11）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aa4c_store::Store;
use aa4c_types::{CoreEvent, Result, SyncFileEntry, SyncScope};
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast::error::RecvError;

use crate::EventSender;

/// 后台定时全量扫描间隔（SYNC_DESIGN.md §11 已确认默认值）。
const SCAN_INTERVAL: Duration = Duration::from_secs(300);

/// 接收中间状态的临时文件后缀（aa4c-transfer），扫描时跳过未完成文件。
const PART_SUFFIX: &str = ".aa4c-part";

/// 扫描单个共享范围：遍历文件系统，与已有索引比对（mtime+size 未变则复用旧 hash，
/// 否则重新计算 BLAKE3），整体替换 Store 里该范围的索引。
pub(crate) async fn scan_scope(store: &Store, scope: &SyncScope) -> Result<()> {
    let existing = store.list_scope_index(&scope.id).await?;
    let mut by_path: HashMap<String, SyncFileEntry> = existing
        .into_iter()
        .map(|e| (e.rel_path.clone(), e))
        .collect();

    let found = walk(Path::new(&scope.local_path)).await?;
    let mut entries = Vec::with_capacity(found.len());
    for (rel_path, abs_path, size, mtime) in found {
        let hash = match by_path.remove(&rel_path) {
            Some(old) if old.size == size && old.mtime == mtime => old.hash,
            _ => Some(hash_file(&abs_path).await?),
        };
        entries.push(SyncFileEntry {
            scope_id: scope.id.clone(),
            rel_path,
            size,
            mtime,
            hash,
            present_local: true,
        });
    }

    store.replace_scope_index(&scope.id, entries).await
}

/// 重新扫描全部共享范围。
pub(crate) async fn rescan_all(store: &Store) -> Result<()> {
    for scope in store.list_sync_scopes().await? {
        scan_scope(store, &scope).await?;
    }
    Ok(())
}

/// 启动后台扫描循环：每 [`SCAN_INTERVAL`] 全量重扫一次；任意一次传输完成
/// （送达 Inbox 或别处）也会立刻触发一次重扫。扫描完成后广播
/// `CoreEvent::SyncIndexUpdated`，UI 据此刷新统一文件视图。
pub(crate) fn spawn_background_scan(store: Store, events: EventSender) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SCAN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // 第一下立即触发，跳过（启动时已扫描过一次）
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                event = sub.recv() => match event {
                    Ok(CoreEvent::TransferDone { .. }) => {}
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                },
            }
            if let Err(e) = rescan_all(&store).await {
                tracing::warn!(error = %e, "sync scan failed");
                continue;
            }
            let _ = events.send(CoreEvent::SyncIndexUpdated);
        }
    });
}

/// 递归列出范围内文件（跳过隐藏文件/目录、符号链接与未完成的接收临时文件）。
/// 返回 (rel_path, 绝对路径, size, mtime 毫秒)。
async fn walk(root: &Path) -> Result<Vec<(String, PathBuf, u64, i64)>> {
    let mut out = Vec::new();
    if !tokio::fs::try_exists(root).await.unwrap_or(false) {
        return Ok(out); // 范围目录已被移走：索引随之清空，不报错
    }
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            continue; // 目录在扫描过程中消失，跳过
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(PART_SUFFIX) {
                continue;
            }
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_symlink() {
                continue;
            } else if meta.is_dir() {
                stack.push((entry.path(), rel));
            } else if meta.is_file() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                    .unwrap_or(0);
                out.push((rel, entry.path(), meta.len(), mtime));
            }
        }
    }
    Ok(out)
}

/// 流式计算文件 BLAKE3（避免大文件占内存，呼应 aa4c-transfer 的做法）。
async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_types::ScopeKind;

    #[tokio::test]
    async fn scan_picks_up_additions_changes_and_removals() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let root = dir.path().join("scope");
        tokio::fs::create_dir_all(root.join("sub")).await.unwrap();
        tokio::fs::write(root.join("a.txt"), b"hello")
            .await
            .unwrap();
        tokio::fs::write(root.join("sub/b.txt"), b"world!")
            .await
            .unwrap();
        tokio::fs::write(root.join(format!("c.bin{PART_SUFFIX}")), b"partial")
            .await
            .unwrap();

        let scope = store
            .upsert_sync_scope(ScopeKind::Folder, &root.to_string_lossy())
            .await
            .unwrap();
        scan_scope(&store, &scope).await.unwrap();

        let mut idx = store.list_scope_index(&scope.id).await.unwrap();
        idx.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        let rels: Vec<_> = idx.iter().map(|e| e.rel_path.as_str()).collect();
        // 未完成的 .aa4c-part 文件被跳过
        assert_eq!(rels, vec!["a.txt", "sub/b.txt"]);
        assert!(idx[0].hash.is_some());
        let a_hash_before = idx[0].hash.clone();

        // 删除一个、修改一个、新增一个，重新扫描应整体反映
        tokio::fs::remove_file(root.join("sub/b.txt"))
            .await
            .unwrap();
        tokio::fs::write(root.join("a.txt"), b"hello, changed")
            .await
            .unwrap();
        tokio::fs::write(root.join("d.txt"), b"new file")
            .await
            .unwrap();
        scan_scope(&store, &scope).await.unwrap();

        let mut idx2 = store.list_scope_index(&scope.id).await.unwrap();
        idx2.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        let rels2: Vec<_> = idx2.iter().map(|e| e.rel_path.as_str()).collect();
        assert_eq!(rels2, vec!["a.txt", "d.txt"]);
        assert_ne!(
            idx2[0].hash, a_hash_before,
            "content changed → hash recomputed"
        );
    }

    #[tokio::test]
    async fn scan_reuses_hash_when_mtime_and_size_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let root = dir.path().join("scope");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(root.join("a.txt"), b"stable")
            .await
            .unwrap();

        let scope = store
            .upsert_sync_scope(ScopeKind::Folder, &root.to_string_lossy())
            .await
            .unwrap();
        scan_scope(&store, &scope).await.unwrap();
        let first = store.list_scope_index(&scope.id).await.unwrap();

        // 内容不变、mtime/size 不变：第二次扫描复用旧 hash（不重新打开文件计算）
        scan_scope(&store, &scope).await.unwrap();
        let second = store.list_scope_index(&scope.id).await.unwrap();
        assert_eq!(first[0].hash, second[0].hash);
    }

    #[tokio::test]
    async fn scan_missing_root_clears_index_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let root = dir.path().join("gone");

        let scope = store
            .upsert_sync_scope(ScopeKind::Folder, &root.to_string_lossy())
            .await
            .unwrap();
        scan_scope(&store, &scope).await.unwrap();
        assert!(store.list_scope_index(&scope.id).await.unwrap().is_empty());
    }
}
