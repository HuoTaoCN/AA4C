//! Core 编排方法：Tauri §9 的 11 个 Command 在此有一一对应的实现，
//! 使 Tauri 层只做参数搬运与错误映射，端到端冒烟测试可直接驱动 Core。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use aa4c_discovery::DiscoveryService;
use aa4c_identity::Identity;
use aa4c_store::Store;
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceId, DeviceInfo, Result, ScopeKind, Settings, SyncFileEntry,
    SyncScope, TaskId, TransferTask, TrustLevel, UnifiedFile,
};

use crate::{server_link, settings, sync_exchange, sync_index, unified, Core};

/// 远程索引「新鲜」窗口（毫秒）：略大于 `sync_exchange::REMOTE_REFRESH_INTERVAL`（30s）
/// 的 3 倍，容忍一次没赶上的周期仍不判离线（里程碑 C4，见 `list_unified_files`）。
const REMOTE_INDEX_FRESH_WINDOW_MS: i64 = 90_000;

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
                let _ = sync_exchange::fetch_one(
                    &self.store,
                    &self.discovery,
                    &self.identity,
                    &self.self_info.name,
                    &self.save_dir_fallback,
                    &self.transfer,
                    &self.events,
                    device_id,
                )
                .await;
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

    /// 立即触发常驻连接重新注册（不阻塞调用方；未开启/未配置/失败都只记日志，见
    /// `server_link`）。设置变更（刚打开 `enable_remote` / 服务器地址变化）、解除配对
    /// 等需要「立即生效」的操作都调用它——`register_notify` 唤醒常驻连接跳过
    /// `IDLE_POLL`/续约窗口，立刻用最新的设置/允许名单重新 `Register`（里程碑 C3；
    /// 不再另开一次性连接，理由见 `server_link` 模块文档）。
    fn nudge_register(&self) {
        self.register_notify.notify_one();
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
        let remote_index = self.store.list_remote_index().await?;
        let now = now_ms();
        // 在线判定（CONNECT_DESIGN.md §6，里程碑 C4）：mDNS 在线 **或** 最近一次远程索引
        // 同步仍在新鲜窗口内——远程设备靠周期定时器同步（见 sync_exchange 模块文档），
        // 「最近同步成功过」是它「当时确实可达」的直接证据，比另起一次实时探测更省成本。
        // 注意这不是"绝对保真"：窗口内设备完全可能已经掉线（NAT 变动等），拉取失败时
        // 前端应给温和提示 + 可重试，不让黄色变成谎言（同一节引用的既有设计原则）。
        let remote_fresh: HashSet<DeviceId> = remote_index
            .iter()
            .filter(|r| now - r.seen_at <= REMOTE_INDEX_FRESH_WINDOW_MS)
            .map(|r| r.device_id.clone())
            .collect();
        let remote: Vec<unified::RemoteEntry> = remote_index
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
        let mut online: HashSet<DeviceId> = remote_fresh;
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

    /// 手动与全部完全信任设备刷新一次跨设备索引（里程碑 3；里程碑 C4 起覆盖远程设备）。
    pub async fn refresh_remote_index(&self) -> Result<()> {
        sync_exchange::refresh_all_full_trust(
            &self.store,
            &self.discovery,
            &self.identity,
            &self.self_info.name,
            &self.save_dir_fallback,
            &self.transfer,
            &self.events,
        )
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

        // 挑一台「我的设备」（完全信任）持有者：mDNS 在线的排前面（大概率直连更快），
        // 但不再要求必须在线快照里——远程持有者一样试（`resolve_peer` 走完整连接阶梯，
        // 解析不出地址也交给 `transfer.fetch_file` 落中继兜底，里程碑 C4）。
        let online_ids: HashSet<DeviceId> =
            self.discovery.devices().into_iter().map(|d| d.id).collect();
        let mut candidates: Vec<DeviceId> = holders.into_iter().collect();
        candidates.sort_by_key(|id| !online_ids.contains(id));

        for holder_id in candidates {
            let is_full = self
                .store
                .get_device(&holder_id)
                .await?
                .map(|d| d.trusted && d.trust_level == TrustLevel::Full)
                .unwrap_or(false);
            if is_full {
                let peer = self.resolve_peer(&holder_id).await?;
                return self.transfer.fetch_file(&peer, rel_path, save_dir).await;
            }
        }
        Err(Aa4cError::Protocol("没有完全信任的设备持有这个文件".into()))
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
        let addr = resolve_addr(
            &self.store,
            &self.discovery,
            &self.identity,
            &self.self_info.name,
            &self.save_dir_fallback,
            device_id,
        )
        .await;
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
}

/// 综合 mDNS 在线快照 → 落库最后地址 → 向自己配置的服务器 Lookup 解析出一个可尝试连接
/// 的地址（CONNECT_DESIGN.md §3.4/§6）。里程碑 C4：抽成自由函数，`resolve_peer`（发送/
/// 按需拉取文件）与 `sync_exchange`（远程索引同步）共用同一套阶梯，不再各自维护一份、
/// 容易跑偏（此前 `fetch_file`/`sync_exchange` 都只查 mDNS，是本里程碑要补的缺口）。
/// 三档都没解析出来时返回 `None`——调用方仍可能靠中继兜底（见
/// `aa4c_transfer::TransferService::dial`），不是「设备不可达」的终审。
pub(crate) async fn resolve_addr(
    store: &Store,
    discovery: &DiscoveryService,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    device_id: &DeviceId,
) -> Option<std::net::SocketAddr> {
    if let Some(addr) = discovery
        .devices()
        .into_iter()
        .find(|d| &d.id == device_id)
        .and_then(|d| d.addr)
    {
        return Some(addr);
    }
    if let Some(rec) = store.get_device(device_id).await.ok().flatten() {
        if let Some(addr) = rec.last_addr.as_deref().and_then(|s| s.parse().ok()) {
            return Some(addr);
        }
    }
    remote_lookup(store, identity, fallback_name, fallback_save_dir, device_id).await
}

/// 向自己配置的服务器查一次对端端点；未开启远程 / 未配置 / 查询失败 / 无结果都静默
/// 返回 `None`（不阻断 [`resolve_addr`] 的其余判定，见上）。
async fn remote_lookup(
    store: &Store,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    device_id: &DeviceId,
) -> Option<std::net::SocketAddr> {
    let settings = settings::load(store, fallback_name, fallback_save_dir)
        .await
        .ok()?;
    if !settings.enable_remote {
        return None;
    }
    let server_url = settings.server_url?;
    match server_link::lookup_once(identity, &server_url, device_id).await {
        Ok(endpoints) => endpoints.into_iter().next(),
        Err(e) => {
            tracing::debug!(error = %e, "remote lookup failed");
            None
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
