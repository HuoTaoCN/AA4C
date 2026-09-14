//! Tauri Command 集（API_DESIGN.md §9.1）与事件 payload 映射（§9.2）。
//!
//! 每个 Command 仅做参数转换并委托给 `Core`；错误统一映射为 `{ code, message }`。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_core::Core;
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceInfo, LocalServerStatus, PendingIntroduction, Settings, Share,
    ShareAccess, SyncConflict, SyncFileEntry, SyncScope, TransferTask, TrustLevel, UnifiedFile,
};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

/// Command 失败时返回给前端的统一形状（错误码取 `Aa4cError` 变体名）。
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl From<Aa4cError> for CommandError {
    fn from(e: Aa4cError) -> Self {
        Self {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[tauri::command]
pub async fn get_self_device(core: State<'_, Arc<Core>>) -> CmdResult<DeviceInfo> {
    Ok(core.self_info())
}

#[tauri::command]
pub async fn list_devices(core: State<'_, Arc<Core>>) -> CmdResult<Vec<DeviceInfo>> {
    Ok(core.list_devices().await?)
}

#[tauri::command]
pub async fn start_pairing(core: State<'_, Arc<Core>>, device_id: String) -> CmdResult<String> {
    Ok(core.start_pairing(&device_id).await?)
}

#[tauri::command]
pub async fn confirm_pairing(
    core: State<'_, Arc<Core>>,
    session_id: String,
    accept: bool,
) -> CmdResult<()> {
    Ok(core.confirm_pairing(&session_id, accept).await?)
}

#[tauri::command]
pub async fn unpair_device(core: State<'_, Arc<Core>>, device_id: String) -> CmdResult<()> {
    Ok(core.unpair_device(&device_id).await?)
}

#[tauri::command]
pub async fn set_trust_level(
    core: State<'_, Arc<Core>>,
    device_id: String,
    level: String,
) -> CmdResult<()> {
    let level: TrustLevel = level.parse()?;
    Ok(core.set_trust_level(&device_id, level).await?)
}

// —— 信任传递 / 引荐（TRUST_DESIGN.md §5，里程碑 R2）——

#[tauri::command]
pub async fn list_pending_introductions(
    core: State<'_, Arc<Core>>,
) -> CmdResult<Vec<PendingIntroduction>> {
    Ok(core.list_pending_introductions().await?)
}

#[tauri::command]
pub async fn confirm_introduction(core: State<'_, Arc<Core>>, device_id: String) -> CmdResult<()> {
    Ok(core.confirm_introduction(&device_id).await?)
}

#[tauri::command]
pub async fn dismiss_introduction(core: State<'_, Arc<Core>>, device_id: String) -> CmdResult<()> {
    Ok(core.dismiss_introduction(&device_id).await?)
}

#[tauri::command]
pub async fn refresh_introductions(core: State<'_, Arc<Core>>) -> CmdResult<()> {
    Ok(core.refresh_introductions().await?)
}

/// 内置服务器状态（TRUST_DESIGN.md §6.3，里程碑 R4）：含给别的设备填的完整地址，
/// 以及那个地址「靠不靠得住」（`reach`）。
#[tauri::command]
pub async fn local_server_status(core: State<'_, Arc<Core>>) -> CmdResult<LocalServerStatus> {
    Ok(core.local_server_status().await?)
}

#[tauri::command]
pub async fn send_files(
    core: State<'_, Arc<Core>>,
    device_id: String,
    paths: Vec<String>,
) -> CmdResult<String> {
    let paths = paths.into_iter().map(PathBuf::from).collect();
    Ok(core.send_files(&device_id, paths).await?)
}

#[tauri::command]
pub async fn accept_transfer(
    core: State<'_, Arc<Core>>,
    task_id: String,
    accept: bool,
    save_dir: Option<String>,
) -> CmdResult<()> {
    Ok(core
        .accept_transfer(&task_id, accept, save_dir.map(PathBuf::from))
        .await?)
}

#[tauri::command]
pub async fn cancel_transfer(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.cancel_transfer(&task_id).await?)
}

#[tauri::command]
pub async fn pause_transfer(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.pause_transfer(&task_id).await?)
}

#[tauri::command]
pub async fn resume_transfer(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.resume_transfer(&task_id).await?)
}

#[tauri::command]
pub async fn list_transfers(
    core: State<'_, Arc<Core>>,
    limit: u32,
    offset: u32,
) -> CmdResult<Vec<TransferTask>> {
    Ok(core.list_transfers(limit, offset).await?)
}

#[tauri::command]
pub async fn get_settings(core: State<'_, Arc<Core>>) -> CmdResult<Settings> {
    Ok(core.get_settings().await?)
}

#[tauri::command]
pub async fn update_settings(core: State<'_, Arc<Core>>, settings: Settings) -> CmdResult<()> {
    Ok(core.update_settings(settings).await?)
}

#[tauri::command]
pub async fn list_sync_scopes(core: State<'_, Arc<Core>>) -> CmdResult<Vec<SyncScope>> {
    Ok(core.list_sync_scopes().await?)
}

#[tauri::command]
pub async fn add_sync_scope(core: State<'_, Arc<Core>>, path: String) -> CmdResult<SyncScope> {
    Ok(core.add_sync_scope(PathBuf::from(path)).await?)
}

#[tauri::command]
pub async fn remove_sync_scope(core: State<'_, Arc<Core>>, id: String) -> CmdResult<()> {
    Ok(core.remove_sync_scope(&id).await?)
}

#[tauri::command]
pub async fn list_sync_files(core: State<'_, Arc<Core>>) -> CmdResult<Vec<SyncFileEntry>> {
    Ok(core.list_sync_files().await?)
}

#[tauri::command]
pub async fn rescan_sync(core: State<'_, Arc<Core>>) -> CmdResult<()> {
    Ok(core.rescan_sync().await?)
}

#[tauri::command]
pub async fn list_unified_files(core: State<'_, Arc<Core>>) -> CmdResult<Vec<UnifiedFile>> {
    Ok(core.list_unified_files().await?)
}

#[tauri::command]
pub async fn refresh_remote_index(core: State<'_, Arc<Core>>) -> CmdResult<()> {
    Ok(core.refresh_remote_index().await?)
}

#[tauri::command]
pub async fn fetch_file(
    core: State<'_, Arc<Core>>,
    rel_path: String,
    hash: Option<String>,
) -> CmdResult<String> {
    Ok(core.fetch_file(&rel_path, hash.as_deref()).await?)
}

#[tauri::command]
pub async fn list_conflicts(core: State<'_, Arc<Core>>) -> CmdResult<Vec<SyncConflict>> {
    Ok(core.list_conflicts().await?)
}

#[tauri::command]
pub async fn create_share(
    core: State<'_, Arc<Core>>,
    rel_path: String,
    expires_at: Option<i64>,
) -> CmdResult<Share> {
    Ok(core.create_share(&rel_path, expires_at).await?)
}

#[tauri::command]
pub async fn list_shares(core: State<'_, Arc<Core>>) -> CmdResult<Vec<Share>> {
    Ok(core.list_shares().await?)
}

#[tauri::command]
pub async fn revoke_share(core: State<'_, Arc<Core>>, id: String) -> CmdResult<()> {
    Ok(core.revoke_share(&id).await?)
}

#[tauri::command]
pub async fn list_share_access(
    core: State<'_, Arc<Core>>,
    share_id: String,
) -> CmdResult<Vec<ShareAccess>> {
    Ok(core.list_share_access(&share_id).await?)
}

#[tauri::command]
pub async fn open_share(core: State<'_, Arc<Core>>, link: String) -> CmdResult<String> {
    Ok(core.open_share(&link, None).await?)
}

/// 把一次调用转给插件（F1，ARCHITECTURE.md 原则 3）。
///
/// 此前这里是 27 个类型化 Command——下载 10、归档 7、AI 5、知识库 5——每个都要
/// 在本文件写一遍、再在 `lib.rs` 的 `generate_handler!` 里登记一遍。它们占了
/// 60 个 Command 里的将近一半，而按 PROJECT_VISION 自己的定义，这些能力都不属于
/// 「设备连成一片」。
///
/// **为什么是一个通用入口而不是按插件分组的类型化 Command**：`generate_handler!`
/// 是编译期展开的，按 feature 分组会让 `lib.rs` 长出一堆 `#[cfg]`；一个入口一次
/// 到位，也给 V1.0 的第三方插件留了真门。代价是参数校验从编译期挪到运行期——
/// 由插件在自己的 `invoke` 里反序列化时负责，错误信息会指名是哪个方法缺哪个字段。
#[tauri::command]
pub async fn plugin_invoke(
    core: State<'_, Arc<Core>>,
    plugin: String,
    method: String,
    payload: Value,
) -> CmdResult<Value> {
    Ok(core.plugin_invoke(&plugin, method, payload).await?)
}

/// 每台已配对设备当下的可达性：能不能连上、走的哪一档、上次连上是什么时候
/// （V0.8「Focus」F2）。首页的设备状态图靠它。
#[tauri::command]
pub async fn list_reachability(
    core: State<'_, Arc<Core>>,
) -> CmdResult<Vec<aa4c_types::DeviceReachability>> {
    Ok(core.list_reachability().await?)
}

/// 本次构建装了哪些插件。前端据此决定「更多」分区显示什么、设置页画哪些表单。
#[tauri::command]
pub async fn plugin_manifest(core: State<'_, Arc<Core>>) -> CmdResult<Vec<Value>> {
    Ok(core.plugin_manifest())
}

/// 把 `CoreEvent` 映射为 §9.2 约定的扁平 payload（统一 camelCase）。
pub fn event_payload(event: &CoreEvent) -> Value {
    match event {
        CoreEvent::DeviceFound(d) | CoreEvent::DeviceUpdated(d) => {
            serde_json::to_value(d).unwrap_or(Value::Null)
        }
        CoreEvent::DeviceLost { id } => json!({ "id": id }),
        CoreEvent::PairingRequest { session_id, peer } => {
            json!({ "sessionId": session_id, "peer": peer })
        }
        CoreEvent::PairingPin { session_id, pin } => {
            json!({ "sessionId": session_id, "pin": pin })
        }
        CoreEvent::PairingResult {
            session_id,
            peer,
            success,
        } => json!({ "sessionId": session_id, "peer": peer, "success": success }),
        CoreEvent::TransferRequest { task } => json!({ "task": task }),
        CoreEvent::TransferConnected { task_id, via } => {
            json!({ "taskId": task_id, "via": via })
        }
        CoreEvent::TransferProgress {
            task_id,
            transferred_bytes,
            total_bytes,
            speed_bps,
            current_file,
        } => json!({
            "taskId": task_id,
            "transferredBytes": transferred_bytes,
            "totalBytes": total_bytes,
            "speedBps": speed_bps,
            "currentFile": current_file,
        }),
        CoreEvent::TransferDone { task_id } => json!({ "taskId": task_id }),
        CoreEvent::TransferPaused { task_id } => json!({ "taskId": task_id }),
        CoreEvent::TransferFailed { task_id, error } => {
            json!({ "taskId": task_id, "error": error })
        }
        CoreEvent::SyncIndexUpdated => Value::Null,
        CoreEvent::IntroductionsUpdated => Value::Null,
        CoreEvent::ReachabilityUpdated => Value::Null,
        CoreEvent::DownloadProgress {
            task_id,
            downloaded_bytes,
            total_bytes,
            speed_bps,
            seeders,
            peers,
            ratio,
        } => {
            // 同 aa4c_types::CoreEvent 的 serde 语义：BT 字段是 None（HTTP 任务）
            // 时整个 key 不出现，不是出现成 null——前端按"字段存在与否"判断这条
            // 任务是不是 BT，不是按"值是不是 null"。
            let mut payload = json!({
                "taskId": task_id,
                "downloadedBytes": downloaded_bytes,
                "totalBytes": total_bytes,
                "speedBps": speed_bps,
            });
            if let Value::Object(map) = &mut payload {
                if let Some(v) = seeders {
                    map.insert("seeders".to_string(), json!(v));
                }
                if let Some(v) = peers {
                    map.insert("peers".to_string(), json!(v));
                }
                if let Some(v) = ratio {
                    map.insert("ratio".to_string(), json!(v));
                }
            }
            payload
        }
        CoreEvent::DownloadDone { task_id, save_path } => {
            json!({ "taskId": task_id, "savePath": save_path })
        }
        CoreEvent::DownloadFailed { task_id, error } => {
            json!({ "taskId": task_id, "error": error })
        }
        CoreEvent::ArchiveApplied {
            entry_id,
            from_path,
            to_path,
            rule_id,
        } => {
            let mut payload = json!({
                "entryId": entry_id,
                "fromPath": from_path,
                "toPath": to_path,
            });
            if let (Value::Object(map), Some(rule_id)) = (&mut payload, rule_id) {
                map.insert("ruleId".to_string(), json!(rule_id));
            }
            payload
        }
        CoreEvent::AiEngineState {
            slot,
            status,
            error,
        } => {
            let mut payload = json!({ "slot": slot, "status": status });
            if let (Value::Object(map), Some(error)) = (&mut payload, error) {
                map.insert("error".to_string(), json!(error));
            }
            payload
        }
        CoreEvent::AiSuggestProgress { done, total } => {
            json!({ "done": done, "total": total })
        }
        CoreEvent::KbIngestProgress {
            source_id,
            done,
            total,
        } => {
            json!({ "sourceId": source_id, "done": done, "total": total })
        }
        CoreEvent::KbAnswerDelta { request_id, delta } => {
            json!({ "requestId": request_id, "delta": delta })
        }
        CoreEvent::KbAnswerDone {
            request_id,
            sources,
            error,
        } => {
            let mut payload = json!({ "requestId": request_id, "sources": sources });
            if let (Value::Object(map), Some(error)) = (&mut payload, error) {
                map.insert("error".to_string(), json!(error));
            }
            payload
        }
    }
}
