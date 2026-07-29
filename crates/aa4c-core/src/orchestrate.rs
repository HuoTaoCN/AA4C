//! Core 编排方法：Tauri §9 的 11 个 Command 在此有一一对应的实现，
//! 使 Tauri 层只做参数搬运与错误映射，端到端冒烟测试可直接驱动 Core。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

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

    // —— 分享链接（CONNECT_DESIGN.md §7/§8，里程碑 C6）——

    /// 生成一条新分享：`rel_path` 必须落在某个共享范围内——复用 `resolve_shared` 校验，
    /// 不接受任意路径（CONNECT_DESIGN.md §7.1「分享目标必须落在共享范围内」）。
    /// `expires_at` 为空 = 长期有效。
    pub async fn create_share(
        &self,
        rel_path: &str,
        expires_at: Option<i64>,
    ) -> Result<aa4c_types::Share> {
        if unified::resolve_shared(&self.store, rel_path)
            .await?
            .is_none()
        {
            return Err(Aa4cError::Protocol("这个路径不在任何共享范围内".into()));
        }
        let token = generate_token();
        let mut share = self
            .store
            .insert_share(&token, rel_path, expires_at)
            .await?;
        share.link = self.share_link(&token).await;
        Ok(share)
    }

    /// 列出全部分享（含完整链接，管理页用）。
    pub async fn list_shares(&self) -> Result<Vec<aa4c_types::Share>> {
        let mut shares = self.store.list_shares().await?;
        for share in &mut shares {
            share.link = self.share_link(&share.token).await;
        }
        Ok(shares)
    }

    /// 吊销一条分享（置 revoked，保留记录供审计）。
    pub async fn revoke_share(&self, id: &str) -> Result<()> {
        self.store.revoke_share(id).await
    }

    /// 某条分享的访问记录（可选功能，CONNECT_DESIGN.md §8）。
    pub async fn list_share_access(&self, share_id: &str) -> Result<Vec<aa4c_types::ShareAccess>> {
        self.store.list_share_access(share_id).await
    }

    /// 打开一个分享链接：解析 payload → 走连接阶梯解析地址 → 拉取内容
    /// （CONNECT_DESIGN.md §7.2，里程碑 C6）。**不要求本机已与分享方配对**——token
    /// 本身就是访问能力，见 `aa4c_transfer::TransferService::open_share` 文档。
    pub async fn open_share(&self, link: &str, save_dir: Option<PathBuf>) -> Result<TaskId> {
        let parsed = aa4c_types::ShareLink::parse(link)?;
        let addr = resolve_share_host_addr(
            &self.store,
            &self.discovery,
            &self.identity,
            &self.self_info.name,
            &self.save_dir_fallback,
            &parsed.host_id,
            parsed.host_server.as_deref(),
        )
        .await;
        self.transfer
            .open_share(&parsed.host_id, addr, parsed.token, save_dir)
            .await
    }

    /// 拼一条完整可分享链接：本机 device_id + 当前配置的服务器地址（未开启远程时不带，
    /// 避免暗示一个没在实际生效的服务器——见 `aa4c_types::Share::link` 文档）。
    async fn share_link(&self, token: &str) -> String {
        let host_server = self
            .get_settings()
            .await
            .ok()
            .and_then(|s| s.enable_remote.then_some(s.server_url).flatten());
        aa4c_types::ShareLink {
            host_id: self.identity.device_id().clone(),
            token: token.to_string(),
            host_server,
        }
        .encode()
    }

    /// 把 device_id 解析为可发送的 DeviceInfo：mDNS 在线快照（含实时地址）→ 落库最后
    /// 地址 → 查对端自己的服务器（`server_hint`）→ 查自己配置的服务器（详见
    /// [`resolve_addr`] 文档，CONNECT_DESIGN.md §3.4，里程碑 C2/后续 gap 补完）。
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

    // —— 下载中心（DOWNLOAD_DESIGN.md，里程碑 D1）——

    /// `self.download` 为 `None` 时统一报 `Unavailable`——本平台/构建未接入下载能力
    /// （与"接入了但 aria2c 起不来"是两种不同的不可用，后者由 `DownloadService`
    /// 内部处理，同样会以 `Unavailable` 报出，前端不需要区分这两种情况）。
    fn download_service(&self) -> Result<&Arc<aa4c_download::DownloadService>> {
        self.download.as_ref().ok_or_else(|| {
            Aa4cError::Unavailable("download capability not available on this build".into())
        })
    }

    /// 新建一条下载任务（D1 只接受 HTTP/HTTPS/FTP 直链）。
    pub async fn add_download(&self, url: String) -> Result<TaskId> {
        self.download_service()?.add(url).await
    }

    pub async fn pause_download(&self, id: TaskId) -> Result<()> {
        self.download_service()?.pause(id).await
    }

    pub async fn resume_download(&self, id: TaskId) -> Result<()> {
        self.download_service()?.resume(id).await
    }

    pub async fn cancel_download(&self, id: TaskId) -> Result<()> {
        self.download_service()?.cancel(id).await
    }

    /// 按创建时间倒序列出全部下载任务。
    pub async fn list_downloads(&self) -> Result<Vec<aa4c_types::DownloadTask>> {
        self.download_service()?.list().await
    }

    /// 批量操作（D3，DOWNLOAD_DESIGN.md §6/§9）：全部暂停/全部继续/清除已完成
    /// 记录。返回值是"实际生效的数量"，薄薄一层转发到 `DownloadService`，具体
    /// 的"单个任务失败只跳过、不中断整体"取舍在那边实现。
    pub async fn pause_all_downloads(&self) -> Result<usize> {
        Ok(self.download_service()?.pause_all().await)
    }

    pub async fn resume_all_downloads(&self) -> Result<usize> {
        Ok(self.download_service()?.resume_all().await)
    }

    pub async fn clear_completed_downloads(&self) -> Result<usize> {
        self.download_service()?.clear_completed().await
    }

    // —— 归档（ARCHIVE_DESIGN.md，里程碑 AI1）——

    pub async fn list_archive_rules(&self) -> Result<Vec<aa4c_types::ArchiveRule>> {
        self.store.list_archive_rules().await
    }

    /// 新建或更新一条规则：`rule.id` 为空串代表新建（core 侧生成 uuid），非空则更新
    /// 同 id 的既有规则（`upsert_archive_rule` 本身就是 upsert 语义）。返回写库后的
    /// 完整规则（含服务器生成的 `created_at`/`updated_at`）。
    pub async fn save_archive_rule(
        &self,
        mut rule: aa4c_types::ArchiveRule,
    ) -> Result<aa4c_types::ArchiveRule> {
        if rule.id.is_empty() {
            rule.id = uuid::Uuid::new_v4().to_string();
        }
        self.store.upsert_archive_rule(&rule).await?;
        self.store
            .list_archive_rules()
            .await?
            .into_iter()
            .find(|r| r.id == rule.id)
            .ok_or_else(|| Aa4cError::Protocol("rule not found immediately after upsert".into()))
    }

    pub async fn delete_archive_rule(&self, id: String) -> Result<()> {
        self.store.delete_archive_rule(&id).await
    }

    pub async fn list_archive_entries(&self) -> Result<Vec<aa4c_types::ArchiveEntry>> {
        self.store.list_archive_entries().await
    }

    /// 批量归档指定路径（归档页/统一文件视图的手动路径，ARCHIVE_DESIGN §2.4）。
    /// `rule_id`：手选某条规则强制应用（不检查该规则的匹配条件）；`target_dir`：
    /// 完全自定义目标目录（不经任何规则，不追加标签）；两者都不给时退回自动匹配
    /// （同下载完成钩子一样的 `apply_rules`，允许对任意文件"现在就跑一遍规则"）。
    /// 单个文件失败只跳过、记录原因，不中断整批（同 D3 批量操作的既有取舍）；
    /// 返回值是"实际归档成功的路径列表"。
    pub async fn archive_files(
        &self,
        paths: Vec<String>,
        rule_id: Option<String>,
        target_dir: Option<String>,
    ) -> Result<Vec<String>> {
        let archive_root = PathBuf::from(self.get_settings().await?.archive_root);
        let mut succeeded = Vec::new();
        for path in paths {
            let source = PathBuf::from(&path);
            let result: Result<Option<PathBuf>> = if let Some(rule_id) = &rule_id {
                crate::archive::engine::apply_selected_rule(
                    &self.store,
                    &self.events,
                    &archive_root,
                    &source,
                    rule_id,
                )
                .await
                .map(|(_, to)| Some(to))
            } else if let Some(target_dir) = &target_dir {
                crate::archive::engine::apply_manual(
                    &self.store,
                    &self.events,
                    &source,
                    &PathBuf::from(target_dir),
                )
                .await
                .map(|(_, to)| Some(to))
            } else {
                crate::archive::engine::apply_rules(
                    &self.store,
                    &self.events,
                    &archive_root,
                    &source,
                )
                .await
                .map(|outcome| match outcome {
                    crate::archive::engine::ApplyOutcome::Applied { to_path, .. } => Some(to_path),
                    crate::archive::engine::ApplyOutcome::NoRuleMatched => None,
                })
            };
            match result {
                Ok(Some(to_path)) => succeeded.push(to_path.to_string_lossy().into_owned()),
                Ok(None) => {
                    tracing::debug!(path = %path, "archive_files: no rule matched, skipped")
                }
                Err(e) => {
                    tracing::warn!(path = %path, error = %e, "archive_files: failed, skipped")
                }
            }
        }
        Ok(succeeded)
    }

    pub async fn undo_archive(&self, log_id: i64) -> Result<()> {
        crate::archive::engine::undo(&self.store, log_id).await
    }

    /// 按时间倒序列出全部移动历史（归档页「最近归档动作」分区用，每条配一个撤销按钮，
    /// 需要 `log_id` 才能调用 `undo_archive`）。
    pub async fn list_archive_log(&self) -> Result<Vec<aa4c_types::ArchiveLogEntry>> {
        self.store.list_archive_log().await
    }
}

/// 综合 mDNS 在线快照 → 落库最后地址 → 查对端自己的服务器（`server_hint`，配对时交换，
/// PROTOCOL.md §17）→ 查本机自己配置的服务器 解析出一个可尝试连接的地址
/// （CONNECT_DESIGN.md §3.4/§6）。里程碑 C4：抽成自由函数，`resolve_peer`（发送/按需
/// 拉取文件）与 `sync_exchange`（远程索引同步）共用同一套阶梯，不再各自维护一份、容易
/// 跑偏（此前 `fetch_file`/`sync_exchange` 都只查 mDNS，是该里程碑要补的缺口）。四档都
/// 没解析出来时返回 `None`——调用方仍可能靠中继兜底（见
/// `aa4c_transfer::TransferService::dial`），不是「设备不可达」的终审。
///
/// `server_hint` 这一档解决的是**跨服务器好友的地址解析**：两个用户各自搭了独立的
/// `aa4c-server`、互为朋友后，只要对端在自己的服务器上注册过且把本机加入了允许名单
/// （配对即互相加入，见 `server_link::run_persistent_session` 的 `allow_list` 构造），
/// 本机凭自己的身份证书直接查对端的服务器就能拿到端点，服务器端不需要额外协作。
/// **仍是已知缩小范围**：这只解决"查到地址"，中继/打洞信令目前仍只会打向本机自己配置的
/// 服务器（`RelayDialer`/`PunchDialer`/`SignalChannel` 未变）——两台互不知情的独立服务器
/// 之间没有公共撮合点，真正的跨服务器中继/打洞需要服务器间联邦协议，是单独的、后置的
/// 项目（CONNECT_DESIGN.md §12"多服务器联邦"）。查到的地址如果是可直连的（对端有公网 IP，
/// 或双方恰好同局域网只是还没被 mDNS 发现），直连依然会成功；对端在 NAT 后不可直连时，
/// 这一档查到的地址会直连失败，仍旧落到中继/打洞兜底（同现状）。
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
    let rec = store.get_device(device_id).await.ok().flatten();
    if let Some(addr) = rec
        .as_ref()
        .and_then(|r| r.last_addr.as_deref())
        .and_then(|s| s.parse().ok())
    {
        return Some(addr);
    }
    if let Some(hint) = rec.as_ref().and_then(|r| r.server_hint.as_deref()) {
        if let Ok(endpoints) = server_link::lookup_once(identity, hint, device_id).await {
            if let Some(addr) = endpoints.into_iter().next() {
                return Some(addr);
            }
        }
    }
    remote_lookup(store, identity, fallback_name, fallback_save_dir, device_id).await
}

/// 分享链接的地址解析：mDNS → 落库最后地址（若碰巧已配对过）→ 直接查 payload 里携带的
/// **对方服务器**（`host_server`，不依赖本机是否配置了同一台服务器）→ 查本机自己配置的
/// 服务器（同一服务器场景，如"我自己的另一台设备"分享给自己）。全部落空仍不是终审——
/// `dial()` 的中继/打洞兜底会再用本机自己配置的服务器试一次（里程碑 C6）。
///
/// **已知缩小范围**：跨服务器的中继/打洞信令目前只会打向本机自己配置的服务器，不会
/// 打向 `host_server`——同 `resolve_addr`/`server_hint` 的既有缺口（见 HANDOFF.md），
/// 分享方与打开方使用不同服务器时，只有二者恰好互为已配对设备且 `host_server` 上能
/// Lookup 到直连地址时才可达；真正意义上的跨服务器中继/打洞留待后续里程碑。
async fn resolve_share_host_addr(
    store: &Store,
    discovery: &DiscoveryService,
    identity: &Identity,
    fallback_name: &str,
    fallback_save_dir: &str,
    host_id: &DeviceId,
    host_server: Option<&str>,
) -> Option<std::net::SocketAddr> {
    if let Some(addr) = discovery
        .devices()
        .into_iter()
        .find(|d| &d.id == host_id)
        .and_then(|d| d.addr)
    {
        return Some(addr);
    }
    if let Some(rec) = store.get_device(host_id).await.ok().flatten() {
        if let Some(addr) = rec.last_addr.as_deref().and_then(|s| s.parse().ok()) {
            return Some(addr);
        }
    }
    if let Some(server) = host_server {
        if let Ok(endpoints) = server_link::lookup_once(identity, server, host_id).await {
            if let Some(addr) = endpoints.into_iter().next() {
                return Some(addr);
            }
        }
    }
    remote_lookup(store, identity, fallback_name, fallback_save_dir, host_id).await
}

/// 生成一个新分享 token：32 字节（256 bit，远超 CONNECT_DESIGN.md §7.1 要求的
/// 128 bit）随机数据 base58 编码。用两个 UUID v4 拼出随机字节，避免为此单独引入一个
/// RNG 依赖——`uuid` 已经是既有依赖，其 v4 生成走 `getrandom`，密码学安全。
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bs58::encode(bytes).into_string()
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
