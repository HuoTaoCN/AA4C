//! 统一文件视图：本机索引 + 远端索引按限定路径归并，判定 🟢/🟡/🔴（SYNC_DESIGN.md §3.4 / §4）。
//!
//! 「限定路径」= `<分组>/<范围内相对路径>`，分组对 Inbox 是「收到的」、对文件夹是其末段名。
//! 收发两端用同一规则生成，于是同名文件能在并集里归并（里程碑 3 先按 rel_path 归并，
//! 同路径不同内容的冲突拆分留给里程碑 5）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use aa4c_proto::IndexItem;
use aa4c_store::Store;
use aa4c_types::{DeviceId, Result, ScopeKind, SyncScope, SyncStatus, UnifiedFile};

/// 范围在统一视图里的分组名：Inbox 固定「收到的」，文件夹取本地路径末段。
pub(crate) fn group_name(scope: &SyncScope) -> String {
    if scope.kind == ScopeKind::Inbox {
        return "收到的".to_string();
    }
    scope
        .local_path
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&scope.local_path)
        .to_string()
}

/// 把（范围, 范围内相对路径）拼成统一视图的限定路径。
pub(crate) fn qualify(group: &str, rel_path: &str) -> String {
    format!("{group}/{rel_path}")
}

/// 本机全部共享范围的限定索引条目，供应答 `IndexRequest`（只含元数据）。
pub(crate) async fn local_shared_items(store: &Store) -> Result<Vec<IndexItem>> {
    let scopes = store.list_sync_scopes().await?;
    let groups: HashMap<String, String> = scopes
        .iter()
        .map(|s| (s.id.clone(), group_name(s)))
        .collect();
    let files = store.list_all_sync_files().await?;
    let mut items = Vec::with_capacity(files.len());
    for f in files {
        let Some(group) = groups.get(&f.scope_id) else {
            continue;
        };
        items.push(IndexItem {
            rel_path: qualify(group, &f.rel_path),
            size: f.size,
            hash: f.hash,
        });
    }
    Ok(items)
}

/// 把统一视图的限定展示路径解析回本机共享文件（按需拉取的服务端，里程碑 4）。
///
/// 顶层段匹配某共享范围的分组名，其余为范围内相对路径。**只解析我们自己已索引（已对外
/// 广播）的条目**——既是隐私/安全边界（绝不按对端任意路径读盘、天然挡掉 `..` 穿越），
/// 也确保拉取的就是统一视图里看到的那一份。返回（绝对路径, 当前大小）。
pub(crate) async fn resolve_shared(
    store: &Store,
    display_rel: &str,
) -> Result<Option<(PathBuf, u64)>> {
    let Some((group, rest)) = display_rel.split_once('/') else {
        return Ok(None);
    };
    if rest.is_empty() {
        return Ok(None);
    }
    for scope in store.list_sync_scopes().await? {
        if group_name(&scope) != group {
            continue;
        }
        let idx = store.list_scope_index(&scope.id).await?;
        if let Some(entry) = idx.iter().find(|e| e.rel_path == rest) {
            let abs = Path::new(&scope.local_path).join(rest);
            // 文件可能在广播后又变化：现取实时大小，取不到则回落索引里的值
            let size = tokio::fs::metadata(&abs)
                .await
                .map(|m| m.len())
                .unwrap_or(entry.size);
            return Ok(Some((abs, size)));
        }
    }
    Ok(None)
}

/// 本机一条限定索引（统一视图归并的左侧输入）。
pub(crate) struct LocalEntry {
    pub rel_path: String,
    pub size: u64,
    pub hash: Option<String>,
}

/// 一条远端索引（归并的右侧输入）。
pub(crate) struct RemoteEntry {
    pub device_id: DeviceId,
    pub rel_path: String,
    pub size: u64,
    pub hash: Option<String>,
}

#[derive(Default)]
struct Accum {
    size: u64,
    hash: Option<String>,
    has_local: bool,
    online_holders: Vec<String>,
    offline_holders: Vec<String>,
}

/// 冲突时给基准路径加序号：第 1 份保留原名，其余在文件名扩展名前插「 (n)」
/// （`收到的/报告.pdf` → `收到的/报告 (2).pdf`）。
fn numbered(base: &str, seq: usize) -> String {
    if seq <= 1 {
        return base.to_string();
    }
    let (dir, name) = match base.rfind('/') {
        Some(i) => (&base[..=i], &base[i + 1..]),
        None => ("", base),
    };
    let numbered = match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({}){}", &name[..dot], seq, &name[dot..]),
        _ => format!("{name} ({seq})"),
    };
    format!("{dir}{numbered}")
}

/// 纯函数归并：本机条目 + 远端条目 + 当前在线设备集合 + 设备名映射 → 统一视图。
///
/// 先按限定基准路径分组，再按 hash 拆分版本（SYNC_DESIGN.md §3.4 / §8）：
/// - 同一路径只有一个 hash → 单条目，原名；
/// - 同一路径多个不同 hash → 并列多条目、加序号区分、各带 `conflict=true`。
///
/// 每条目着色：本机有 → 🟢 绿；本机没有但有在线持有设备 → 🟡 黄；仅离线设备有 → 🔴 红。
pub(crate) fn merge(
    local: Vec<LocalEntry>,
    remote: Vec<RemoteEntry>,
    online: &HashSet<DeviceId>,
    names: &HashMap<DeviceId, String>,
) -> Vec<UnifiedFile> {
    // (基准路径 → (hash 键 → 累积))；hash 键空串代表无 hash（正常不出现）
    let mut map: BTreeMap<String, BTreeMap<String, Accum>> = BTreeMap::new();
    for l in local {
        let hk = l.hash.clone().unwrap_or_default();
        let acc = map.entry(l.rel_path).or_default().entry(hk).or_default();
        acc.has_local = true;
        acc.size = l.size;
        acc.hash = l.hash;
    }
    for r in remote {
        let hk = r.hash.clone().unwrap_or_default();
        let acc = map.entry(r.rel_path).or_default().entry(hk).or_default();
        acc.size = r.size;
        if acc.hash.is_none() {
            acc.hash = r.hash;
        }
        let name = names
            .get(&r.device_id)
            .cloned()
            .unwrap_or_else(|| "其他设备".to_string());
        if online.contains(&r.device_id) {
            if !acc.online_holders.contains(&name) {
                acc.online_holders.push(name);
            }
        } else if !acc.offline_holders.contains(&name) {
            acc.offline_holders.push(name);
        }
    }

    let mut out = Vec::new();
    for (base, versions) in map {
        let conflict = versions.len() > 1;
        // 版本排序（决定序号）：本机有优先，其次有在线持有者，再按 hash 键稳定排序
        let mut vs: Vec<Accum> = versions.into_values().collect();
        vs.sort_by(|a, b| {
            b.has_local
                .cmp(&a.has_local)
                .then((!b.online_holders.is_empty()).cmp(&(!a.online_holders.is_empty())))
                .then(a.hash.cmp(&b.hash))
        });
        for (i, acc) in vs.into_iter().enumerate() {
            let status = if acc.has_local {
                SyncStatus::Local
            } else if !acc.online_holders.is_empty() {
                SyncStatus::Online
            } else {
                SyncStatus::Offline
            };
            let mut holders = Vec::new();
            if acc.has_local {
                holders.push("这台设备".to_string());
            }
            holders.extend(acc.online_holders);
            holders.extend(acc.offline_holders);
            out.push(UnifiedFile {
                rel_path: numbered(&base, i + 1),
                base_path: base.clone(),
                size: acc.size,
                hash: acc.hash,
                status,
                holders,
                conflict,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(rel: &str) -> LocalEntry {
        LocalEntry {
            rel_path: rel.into(),
            size: 10,
            hash: Some("h".into()),
        }
    }
    fn remote(dev: &str, rel: &str) -> RemoteEntry {
        RemoteEntry {
            device_id: dev.into(),
            rel_path: rel.into(),
            size: 10,
            hash: Some("h".into()),
        }
    }

    #[test]
    fn group_name_for_inbox_and_folder() {
        let inbox = SyncScope {
            id: "1".into(),
            kind: ScopeKind::Inbox,
            local_path: "/Users/x/Downloads/AA4C".into(),
            created_at: 0,
        };
        let folder = SyncScope {
            id: "2".into(),
            kind: ScopeKind::Folder,
            local_path: "/Users/x/Photos/".into(),
            created_at: 0,
        };
        assert_eq!(group_name(&inbox), "收到的");
        assert_eq!(group_name(&folder), "Photos");
    }

    #[test]
    fn merge_assigns_green_yellow_red() {
        let names: HashMap<DeviceId, String> = [
            ("devA".to_string(), "客厅电脑".to_string()),
            ("devB".to_string(), "旧手机".to_string()),
        ]
        .into_iter()
        .collect();
        let online: HashSet<DeviceId> = ["devA".to_string()].into_iter().collect();

        let out = merge(
            vec![local("收到的/local.jpg")],
            vec![
                // 本机也有的，远端在线 → 仍绿（本机优先）
                remote("devA", "收到的/local.jpg"),
                // 仅在线远端有 → 黄
                remote("devA", "项目/online.rs"),
                // 仅离线远端有 → 红
                remote("devB", "项目/offline.rs"),
            ],
            &online,
            &names,
        );
        let by: HashMap<&str, &UnifiedFile> =
            out.iter().map(|f| (f.rel_path.as_str(), f)).collect();

        assert_eq!(by["收到的/local.jpg"].status, SyncStatus::Local);
        assert_eq!(by["收到的/local.jpg"].holders[0], "这台设备");
        assert!(by["收到的/local.jpg"]
            .holders
            .contains(&"客厅电脑".to_string()));

        assert_eq!(by["项目/online.rs"].status, SyncStatus::Online);
        assert_eq!(by["项目/online.rs"].holders, vec!["客厅电脑"]);

        assert_eq!(by["项目/offline.rs"].status, SyncStatus::Offline);
        assert_eq!(by["项目/offline.rs"].holders, vec!["旧手机"]);
        // 单 hash → 非冲突，展示路径 == 基准路径
        assert!(!by["项目/online.rs"].conflict);
        assert_eq!(by["项目/online.rs"].base_path, "项目/online.rs");
    }

    #[test]
    fn numbered_inserts_before_extension() {
        assert_eq!(numbered("收到的/报告.pdf", 1), "收到的/报告.pdf");
        assert_eq!(numbered("收到的/报告.pdf", 2), "收到的/报告 (2).pdf");
        assert_eq!(numbered("no_ext", 2), "no_ext (2)");
        assert_eq!(numbered("dir/.hidden", 2), "dir/.hidden (2)");
    }

    #[test]
    fn merge_splits_same_path_different_hash_into_numbered_variants() {
        let names: HashMap<DeviceId, String> = [
            ("devA".to_string(), "客厅电脑".to_string()),
            ("devB".to_string(), "旧手机".to_string()),
        ]
        .into_iter()
        .collect();
        let online: HashSet<DeviceId> = ["devA".to_string()].into_iter().collect();

        // 本机有 h1；devA(在线) 有 h2；devB(离线) 有 h3 —— 同一路径三个不同版本
        let out = merge(
            vec![LocalEntry {
                rel_path: "收到的/报告.pdf".into(),
                size: 10,
                hash: Some("h1".into()),
            }],
            vec![
                RemoteEntry {
                    device_id: "devA".into(),
                    rel_path: "收到的/报告.pdf".into(),
                    size: 20,
                    hash: Some("h2".into()),
                },
                RemoteEntry {
                    device_id: "devB".into(),
                    rel_path: "收到的/报告.pdf".into(),
                    size: 30,
                    hash: Some("h3".into()),
                },
            ],
            &online,
            &names,
        );
        assert_eq!(out.len(), 3, "three distinct versions");
        assert!(out.iter().all(|f| f.conflict));
        assert!(out.iter().all(|f| f.base_path == "收到的/报告.pdf"));

        // 排序：本机(绿) 第一（原名），在线(黄) 第二，离线(红) 第三
        assert_eq!(out[0].rel_path, "收到的/报告.pdf");
        assert_eq!(out[0].status, SyncStatus::Local);
        assert_eq!(out[0].hash.as_deref(), Some("h1"));

        assert_eq!(out[1].rel_path, "收到的/报告 (2).pdf");
        assert_eq!(out[1].status, SyncStatus::Online);
        assert_eq!(out[1].hash.as_deref(), Some("h2"));

        assert_eq!(out[2].rel_path, "收到的/报告 (3).pdf");
        assert_eq!(out[2].status, SyncStatus::Offline);
        assert_eq!(out[2].hash.as_deref(), Some("h3"));
    }
}
