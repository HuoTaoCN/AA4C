//! 统一文件视图：本机索引 + 远端索引按限定路径归并，判定 🟢/🟡/🔴（SYNC_DESIGN.md §3.4 / §4）。
//!
//! 「限定路径」= `<分组>/<范围内相对路径>`，分组对 Inbox 是「收到的」、对文件夹是其末段名。
//! 收发两端用同一规则生成，于是同名文件能在并集里归并（里程碑 3 先按 rel_path 归并，
//! 同路径不同内容的冲突拆分留给里程碑 5）。

use std::collections::{BTreeMap, HashMap, HashSet};

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

/// 纯函数归并：本机条目 + 远端条目 + 当前在线设备集合 + 设备名映射 → 统一视图。
///
/// - 本机有 → 🟢 绿（`Local`）；
/// - 本机没有但有在线持有设备 → 🟡 黄（`Online`）；
/// - 本机没有且仅离线设备有 → 🔴 红（`Offline`）。
pub(crate) fn merge(
    local: Vec<LocalEntry>,
    remote: Vec<RemoteEntry>,
    online: &HashSet<DeviceId>,
    names: &HashMap<DeviceId, String>,
) -> Vec<UnifiedFile> {
    let mut map: BTreeMap<String, Accum> = BTreeMap::new();
    for l in local {
        let acc = map.entry(l.rel_path).or_default();
        acc.has_local = true;
        acc.size = l.size;
        acc.hash = l.hash;
    }
    for r in remote {
        let acc = map.entry(r.rel_path).or_default();
        if !acc.has_local && acc.hash.is_none() {
            acc.size = r.size;
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

    map.into_iter()
        .map(|(rel_path, acc)| {
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
            UnifiedFile {
                rel_path,
                size: acc.size,
                hash: acc.hash,
                status,
                holders,
            }
        })
        .collect()
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
    }
}
