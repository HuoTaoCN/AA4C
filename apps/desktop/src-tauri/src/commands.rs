//! Tauri Command 集（API_DESIGN.md §9.1）与事件 payload 映射（§9.2）。
//!
//! 每个 Command 仅做参数转换并委托给 `Core`；错误统一映射为 `{ code, message }`。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_core::Core;
use aa4c_types::{
    Aa4cError, CoreEvent, DeviceInfo, DownloadTask, Settings, Share, ShareAccess, SyncConflict,
    SyncFileEntry, SyncScope, TransferTask, TrustLevel, UnifiedFile,
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

#[tauri::command]
pub async fn add_download(core: State<'_, Arc<Core>>, url: String) -> CmdResult<String> {
    Ok(core.add_download(url).await?)
}

#[tauri::command]
pub async fn pause_download(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.pause_download(task_id).await?)
}

#[tauri::command]
pub async fn resume_download(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.resume_download(task_id).await?)
}

#[tauri::command]
pub async fn cancel_download(core: State<'_, Arc<Core>>, task_id: String) -> CmdResult<()> {
    Ok(core.cancel_download(task_id).await?)
}

#[tauri::command]
pub async fn list_downloads(core: State<'_, Arc<Core>>) -> CmdResult<Vec<DownloadTask>> {
    Ok(core.list_downloads().await?)
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
        CoreEvent::TransferFailed { task_id, error } => {
            json!({ "taskId": task_id, "error": error })
        }
        CoreEvent::SyncIndexUpdated => Value::Null,
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
    }
}
