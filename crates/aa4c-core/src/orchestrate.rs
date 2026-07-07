//! Core 编排方法：Tauri §9 的 11 个 Command 在此有一一对应的实现，
//! 使 Tauri 层只做参数搬运与错误映射，端到端冒烟测试可直接驱动 Core。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, DeviceInfo, Result, ScopeKind, Settings, SyncFileEntry,
    SyncScope, TaskId, TransferTask, TrustLevel, UnifiedFile,
};

use crate::{server_link, settings, sync_exchange, sync_index, unified, Core};

impl Core {
    /// 已发现（在线）+ 已配对（可能离线）设备合并，按 id 去重。
    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut map: BTreeMap<DeviceId, DeviceInfo> = BTreeMap::new();

        // 已配对设备：默认离线，地址取配对记录里的最后地址
        for rec in self.store.list_paired_devices().await? {
            let addr = rec.last_addr.as_deref().and_then(|s| s.parse().ok());
            map.insert(
                rec.id.clone(),
                DeviceInfo {
                    id: rec.id,
                    name: rec.name,
                    platform: rec.platform,
                    version: String::new(),
                    addr,
                    online: false,
                    trusted: true,
                    trust_level: Some(rec.trust_level),
                },
            );
        }

        // 已发现设备：在线，覆盖实时地址 / 名称 / 版本
        for dev in self.discovery.devices() {
            map.entry(dev.id.clone())
                .and_modify(|e| {
                    e.name = dev.name.clone();
                    e.platform = dev.platform;
                    e.version = dev.version.clone();
                    e.addr = dev.addr;
                    e.online = true;
                })
                .or_insert(DeviceInfo {
                    online: true,
                    trusted: false,
                    ..dev
                });
        }

        Ok(map.into_values().collect())
    }

    /// 向某设备发起配对（仅对当前在线/已发现设备有效）。
    pub async fn start_pairing(&self, device_id: &DeviceId) -> Result<String> {
        let peer = self
            .discovery
            .devices()
            .into_iter()
            .find(|d| &d.id == device_id)
            .ok_or_else(|| Aa4cError::DeviceNotFound(device_id.clone()))?;
        self.pairing.start_pairing(&peer).await
    }

    /// 确认 / 拒绝配对（PIN 核对或接受请求）。
    pub async fn confirm_pairing(&self, session_id: &str, accept: bool) -> Result<()> {
        self.pairing.confirm(session_id, accept).await
    }

    /// 解除配对（删除设备及其级联记录）。立即触发一次注册续约，让对方的服务器允许名单
    /// 尽快不再包含本机（CONNECT_DESIGN.md §3.3 吊销，里程碑 C2）。
    pub async fn unpair_device(&self, device_id: &DeviceId) -> Result<()> {
        self.store.remove_device(device_id).await?;
        self.nudge_register();
        Ok(())
    }

    /// 变更设备信任分级（「我的设备」full ⇄「朋友」friend）。
    ///
    /// 降级出 full 时按 SYNC_DESIGN §2 立即清空该设备的远端索引缓存（其条目从统一视图消失），
    /// 本机已落地文件不动；升级为 full 则立即尝试拉一次对端索引。
    pub async fn set_trust_level(&self, device_id: &DeviceId, level: TrustLevel) -> Result<()> {
        self.store.set_trust_level(device_id, level).await?;
        match level {
            TrustLevel::Friend => {
                self.store.clear_remote_index(device_id).await?;
                let _ = self.events.send(CoreEvent::SyncIndexUpdated);
            }
            TrustLevel::Full => {
                if let Some(dev) = self
                    .discovery
                    .devices()
                    .into_iter()
                    .find(|d| &d.id == device_id)
                {
                    let _ =
                        sync_exchange::fetch_one(&self.store, &self.transfer, &self.events, &dev)
                            .await;
                }
            }
        }
        Ok(())
    }

    /// 发起 AA 发送，返回 task_id。
    pub async fn send_files(&self, device_id: &DeviceId, paths: Vec<PathBuf>) -> Result<TaskId> {
        let peer = self.resolve_peer(device_id).await?;
        self.transfer.send(&peer, paths).await
    }

    /// 接收端确认。`save_dir` 为空时回落到当前设置的接收目录。
    pub async fn accept_transfer(
        &self,
        task_id: &TaskId,
        accept: bool,
        save_dir: Option<PathBuf>,
    ) -> Result<()> {
        let dir = match save_dir {
            Some(d) => Some(d),
            None => self
                .get_settings()
                .await
                .ok()
                .map(|s| PathBuf::from(s.save_dir)),
        };
        self.transfer.accept(task_id, accept, dir).await
    }

    /// 取消任务（双方均可）。
    pub async fn cancel_transfer(&self, task_id: &TaskId) -> Result<()> {
        self.transfer.cancel(task_id).await
    }

    /// 分页列出传输记录。
    pub async fn list_transfers(&self, limit: u32, offset: u32) -> Result<Vec<TransferTask>> {
        self.store.list_tasks(limit, offset).await
    }

    /// 读取设置（缺省补齐）。
    pub async fn get_settings(&self) -> Result<Settings> {
        settings::load(&self.store, &self.self_info.name, &self.save_dir_fallback).await
    }

    /// 保存设置；设备名变更时重新广播 mDNS。
    pub async fn update_settings(&self, new: Settings) -> Result<()> {
        let old = self.get_settings().await?;
        settings::save(&self.store, &new).await?;
        if new.device_name != old.device_name {
            self.discovery.rebroadcast(new.device_name).await?;
        }
        if new.save_dir != old.save_dir {
            let inbox = self.store.ensure_inbox_scope(&new.save_dir).await?;
            sync_index::scan_scope(&self.store, &inbox).await?;
            let _ = self.events.send(CoreEvent::SyncIndexUpdated);
        }
        // 刚打开远程 / 服务器地址变了：立即注册一次，不必等下一轮周期轮询才生效
        // （CONNECT_DESIGN.md §3.2，里程碑 C2）。
        if new.enable_remote
            && (new.enable_remote != old.enable_remote || new.server_url != old.server_url)
        {
            self.nudge_register();
        }
        Ok(())
    }

    /// 立即触发一次注册续约（不阻塞调用方；未开启/未配置/失败都只记日志，见 `server_link`）。
    fn nudge_register(&self) {
        server_link::nudge_register(
            self.store.clone(),
            self.identity.clone(),
            self.listen_port,
            self.self_info.name.clone(),
            self.save_dir_fallback.clone(),
        );
    }

    // —— 同步：共享范围 + 本地索引（SYNC_DESIGN.md §3/§6，里程碑 2）——

    /// 列出共享范围（含自动维护的 Inbox）。
    pub async fn list_sync_scopes(&self) -> Result<Vec<SyncScope>> {
        self.store.list_sync_scopes().await
    }

    /// 添加一个同步文件夹，立即扫描一次。
    pub async fn add_sync_scope(&self, local_path: PathBuf) -> Result<SyncScope> {
        let scope = self
            .store
            .upsert_sync_scope(ScopeKind::Folder, &local_path.to_string_lossy())
            .await?;
        sync_index::scan_scope(&self.store, &scope).await?;
        let _ = self.events.send(CoreEvent::SyncIndexUpdated);
        Ok(scope)
    }

    /// 移除一个共享范围（Inbox 不可移除，自动维护）。
    pub async fn remove_sync_scope(&self, id: &str) -> Result<()> {
        let scopes = self.store.list_sync_scopes().await?;
        if scopes
            .iter()
            .any(|s| s.id == id && s.kind == ScopeKind::Inbox)
        {
            return Err(Aa4cError::Protocol("inbox scope cannot be removed".into()));
        }
        self.store.remove_sync_scope(id).await?;
        let _ = self.events.send(CoreEvent::SyncIndexUpdated);
        Ok(())
    }

    /// 本机扫描出的原始文件索引（不含跨设备归并；调试/兼容用）。
    pub async fn list_sync_files(&self) -> Result<Vec<SyncFileEntry>> {
        self.store.list_all_sync_files().await
    }

    /// 统一文件视图（SYNC_DESIGN.md §3.4 / §4 / §8，里程碑 3 + 5）：本机索引 + 远端索引
    /// 按限定路径归并，每条带 🟢/🟡/🔴 状态与持有设备；同名不同 hash 拆成带序号的冲突版本，
    /// 并把当前冲突整体落 `sync_conflicts` 供人工挑选。
    pub async fn list_unified_files(&self) -> Result<Vec<UnifiedFile>> {
        // 本机：按范围分组名限定路径
        let scopes = self.store.list_sync_scopes().await?;
        let groups: HashMap<String, String> = scopes
            .iter()
            .map(|s| (s.id.clone(), unified::group_name(s)))
            .collect();
        let local: Vec<unified::LocalEntry> = self
            .store
            .list_all_sync_files()
            .await?
            .into_iter()
            .filter_map(|f| {
                groups.get(&f.scope_id).map(|g| unified::LocalEntry {
                    rel_path: unified::qualify(g, &f.rel_path),
                    size: f.size,
                    hash: f.hash,
                })
            })
            .collect();

        // 远端：device_id → (在线?, 设备名)
        let remote: Vec<unified::RemoteEntry> = self
            .store
            .list_remote_index()
            .await?
            .into_iter()
            .map(|r| unified::RemoteEntry {
                device_id: r.device_id,
                rel_path: r.rel_path,
                size: r.size,
                hash: r.hash,
            })
            .collect();

        let mut names: HashMap<DeviceId, String> = HashMap::new();
        for rec in self.store.list_paired_devices().await? {
            names.insert(rec.id, rec.name);
        }
        let mut online: HashSet<DeviceId> = HashSet::new();
        for dev in self.discovery.devices() {
            online.insert(dev.id.clone());
            names.insert(dev.id, dev.name);
        }

        let files = unified::merge(local, remote, &online, &names);

        // 探测到的冲突整体落库（每个冲突版本一行；解决后下次刷新自动清掉）
        let conflicts: Vec<(String, String)> = files
            .iter()
            .filter(|f| f.conflict)
            .map(|f| (f.base_path.clone(), f.hash.clone().unwrap_or_default()))
            .collect();
        self.store.replace_conflicts(conflicts).await?;

        Ok(files)
    }

    /// 全部冲突版本记录（一个 `rel_path` 有 ≥2 行即一处冲突，里程碑 5）。
    pub async fn list_conflicts(&self) -> Result<Vec<aa4c_types::SyncConflict>> {
        self.store.list_conflicts().await
    }

    /// 手动触发全部共享范围重新扫描。
    pub async fn rescan_sync(&self) -> Result<()> {
        sync_index::rescan_all(&self.store).await?;
        let _ = self.events.send(CoreEvent::SyncIndexUpdated);
        Ok(())
    }

    /// 手动与当前在线的完全信任设备刷新一次跨设备索引（里程碑 3）。
    pub async fn refresh_remote_index(&self) -> Result<()> {
        sync_exchange::refresh_online(&self.store, &self.transfer, &self.discovery, &self.events)
            .await;
        Ok(())
    }

    /// 按需拉取统一视图里某条目的内容（SYNC_DESIGN.md §4 / §8，里程碑 4 + 5）。
    ///
    /// `rel_path` 是限定**基准**路径（对端认得的真实路径，非加了序号的展示名）；`hash` 指定
    /// 要拉哪个版本（冲突时区分不同版本，`None` 表示不限）。从远端索引里找出持有该（路径,版本）
    /// 的设备，挑一台**在线**且**完全信任**的，复用 ATP 拉取；落地后扫描转绿。
    ///
    /// 落点：按限定路径顶层分组段匹配本机某个共享范围，命中则**落回该范围原结构**
    /// （文件夹来源文件回到原文件夹、原黄条目转绿）；否则回落 Inbox（默认接收目录）。
    pub async fn fetch_file(&self, rel_path: &str, hash: Option<&str>) -> Result<TaskId> {
        let holders: HashSet<DeviceId> = self
            .store
            .list_remote_index()
            .await?
            .into_iter()
            .filter(|r| r.rel_path == rel_path && hash.is_none_or(|h| r.hash.as_deref() == Some(h)))
            .map(|r| r.device_id)
            .collect();
        if holders.is_empty() {
            return Err(Aa4cError::Protocol("没有设备持有这个文件".into()));
        }

        // 落点：顶层分组段命中本机某共享范围 → 落回其目录（保留原结构、原黄条目转绿）；
        // 未命中 → None（transfer 侧回落 Inbox）。
        let save_dir = match rel_path.split_once('/') {
            Some((group, _)) => self
                .store
                .list_sync_scopes()
                .await?
                .into_iter()
                .find(|s| unified::group_name(s) == group)
                .map(|s| PathBuf::from(s.local_path)),
            None => None,
        };

        // 在线快照里挑一台「我的设备」（完全信任）持有者
        for dev in self.discovery.devices() {
            if !holders.contains(&dev.id) {
                continue;
            }
            let is_full = self
                .store
                .get_device(&dev.id)
                .await?
                .map(|d| d.trusted && d.trust_level == TrustLevel::Full)
                .unwrap_or(false);
            if is_full {
                return self.transfer.fetch_file(&dev, rel_path, save_dir).await;
            }
        }
        Err(Aa4cError::Protocol("持有这个文件的设备当前不在线".into()))
    }

    /// 把 device_id 解析为可发送的 DeviceInfo：mDNS 在线快照（含实时地址）→ 落库最后
    /// 地址 → 向自己配置的服务器查一次（CONNECT_DESIGN.md §3.4，里程碑 C2）。
    ///
    /// 远程兜底目前只查**自己配置的服务器**，覆盖「自己的多台设备」这一主场景（它们
    /// 天然共用同一服务器）；跨服务器的好友寻址需要在配对时交换 `devices.server_hint`
    /// 并据此选择服务器去查——这部分线路层交换尚未实现，是已知的、有意缩小的范围
    /// （见 HANDOFF.md），留待后续里程碑随连接阶梯一起补齐。
    async fn resolve_peer(&self, device_id: &DeviceId) -> Result<DeviceInfo> {
        if let Some(dev) = self
            .discovery
            .devices()
            .into_iter()
            .find(|d| &d.id == device_id)
        {
            return Ok(dev);
        }
        let rec = self
            .store
            .get_device(device_id)
            .await?
            .ok_or_else(|| Aa4cError::DeviceNotFound(device_id.clone()))?;
        let mut addr = rec.last_addr.as_deref().and_then(|s| s.parse().ok());
        if addr.is_none() {
            addr = self.remote_lookup(device_id).await;
        }
        Ok(DeviceInfo {
            id: rec.id,
            name: rec.name,
            platform: rec.platform,
            version: String::new(),
            addr,
            online: false,
            trusted: rec.trusted,
            trust_level: Some(rec.trust_level),
        })
    }

    /// 向自己配置的服务器查一次对端端点；未开启远程 / 未配置 / 查询失败 / 无结果都
    /// 静默返回 `None`（不阻断 `resolve_peer` 的其余判定，见上）。
    async fn remote_lookup(&self, device_id: &DeviceId) -> Option<std::net::SocketAddr> {
        let settings = self.get_settings().await.ok()?;
        if !settings.enable_remote {
            return None;
        }
        let server_url = settings.server_url?;
        match server_link::lookup_once(&self.identity, &server_url, device_id).await {
            Ok(endpoints) => endpoints.into_iter().next(),
            Err(e) => {
                tracing::debug!(error = %e, "remote lookup failed");
                None
            }
        }
    }
}
