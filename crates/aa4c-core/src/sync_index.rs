//! 本地文件索引扫描（SYNC_DESIGN.md §3.2、§10 里程碑 2）。
//!
//! 维护策略 = **文件系统实时监听 + 定时扫描兜底**（SYNC_DESIGN.md §3.2 的终态）：
//! - `notify`（带 2s 去抖）监听各共享范围目录，增删改秒级触发重扫；
//! - 每 [`SCAN_INTERVAL`] 定时全量重扫补漏（监听盲区：漏事件、外部挂载、批量操作）；
//! - 任意一次传输完成也追加一次重扫。
//!
//! 哈希惰性计算：先比 mtime+size，疑似变化才重算 BLAKE3。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use aa4c_store::Store;
use aa4c_types::{CoreEvent, Result, SyncFileEntry, SyncScope};
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::EventSender;

/// 后台定时全量扫描间隔（SYNC_DESIGN.md §11 已确认默认值；监听之外的兜底）。
const SCAN_INTERVAL: Duration = Duration::from_secs(300);

/// 文件系统事件去抖窗口：一次保存/批量操作只触发一次重扫。
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

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

/// 启动后台扫描循环：文件系统监听（去抖）/ 定时 [`SCAN_INTERVAL`] / 传输完成 三者
/// 任一触发即全量重扫，完成后广播 `CoreEvent::SyncIndexUpdated`，UI 据此刷新统一视图。
/// 监听目录随共享范围增删自动对齐；监听不可用时静默退化为「定时 + 传输」。
pub(crate) fn spawn_background_scan(store: Store, events: EventSender, stop: CancellationToken) {
    let mut sub = events.subscribe();
    tokio::spawn(async move {
        // 文件系统监听：事件（已去抖）汇入 fs_rx，与定时器 / 传输事件同处一个 select。
        // 监听器创建失败（罕见）不致命——退化为纯「定时 + 传输」触发。
        let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let mut watcher: Option<Debouncer<RecommendedWatcher>> =
            match new_debouncer(WATCH_DEBOUNCE, move |res: DebounceEventResult| {
                if res.is_ok() {
                    let _ = fs_tx.send(());
                }
            }) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "file watcher unavailable, periodic scan only");
                    None
                }
            };
        let mut watched: HashSet<PathBuf> = HashSet::new();

        let mut ticker = tokio::time::interval(SCAN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // 第一下立即触发，跳过（启动时已扫描过一次）
        loop {
            // 每轮把监听目录与当前共享范围对齐（新增文件夹开始监听、移除的停止）
            if let Some(w) = watcher.as_mut() {
                reconcile_watches(w, &mut watched, &store).await;
            }
            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                _ = ticker.tick() => {}
                // 监听不可用时禁用该分支，避免关闭的 channel 让 select 空转
                fs = fs_rx.recv(), if watcher.is_some() => match fs {
                    Some(()) => {}
                    None => break,
                },
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

/// 把监听目录集合对齐到当前共享范围：新增的开始监听、消失的停止监听（幂等）。
async fn reconcile_watches(
    watcher: &mut Debouncer<RecommendedWatcher>,
    watched: &mut HashSet<PathBuf>,
    store: &Store,
) {
    let want: HashSet<PathBuf> = match store.list_sync_scopes().await {
        Ok(scopes) => scopes
            .into_iter()
            .map(|s| PathBuf::from(s.local_path))
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => return, // 读范围失败：保持现有监听不动
    };
    for p in watched.difference(&want).cloned().collect::<Vec<_>>() {
        let _ = watcher.watcher().unwatch(&p);
        watched.remove(&p);
    }
    for p in want.difference(watched).cloned().collect::<Vec<_>>() {
        if watcher
            .watcher()
            .watch(&p, RecursiveMode::Recursive)
            .is_ok()
        {
            watched.insert(p);
        }
    }
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

    #[tokio::test]
    async fn watcher_rescans_on_filesystem_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
        let root = dir.path().join("scope");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let scope = store
            .upsert_sync_scope(ScopeKind::Folder, &root.to_string_lossy())
            .await
            .unwrap();

        let (events, mut rx) = tokio::sync::broadcast::channel(16);
        spawn_background_scan(store.clone(), events, CancellationToken::new());
        // 给首轮 reconcile 一点时间开始监听 root（此时索引仍为空，尚未触发扫描）
        tokio::time::sleep(Duration::from_millis(800)).await;

        // 写入新文件：应经 notify（2s 去抖）触发一次重扫并广播 SyncIndexUpdated
        tokio::fs::write(root.join("new.txt"), b"hi").await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(CoreEvent::SyncIndexUpdated) = rx.recv().await {
                    return;
                }
            }
        })
        .await;
        assert!(got.is_ok(), "文件变更应在超时内触发监听重扫");

        let idx = store.list_scope_index(&scope.id).await.unwrap();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].rel_path, "new.txt");
    }
}
