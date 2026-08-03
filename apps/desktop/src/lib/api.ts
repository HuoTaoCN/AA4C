// 11 个 Tauri Command 的类型化封装（API_DESIGN.md §9.1）。
// 失败时 invoke 会 reject 一个 CommandError（{ code, message }）。

import { invoke } from "@tauri-apps/api/core";
import type {
  AiStatus,
  ArchiveEntry,
  ArchiveLogEntry,
  ArchiveRule,
  CommandError,
  DeviceInfo,
  DownloadTask,
  KbSource,
  KbSourceSummary,
  LocalModel,
  Settings,
  Share,
  ShareAccess,
  Suggestion,
  SyncConflict,
  SyncFileEntry,
  SyncScope,
  TransferTask,
  TrustLevel,
  UnifiedFile,
} from "./types";

export const api = {
  getSelfDevice: () => invoke<DeviceInfo>("get_self_device"),
  listDevices: () => invoke<DeviceInfo[]>("list_devices"),

  startPairing: (deviceId: string) =>
    invoke<string>("start_pairing", { deviceId }),
  confirmPairing: (sessionId: string, accept: boolean) =>
    invoke<void>("confirm_pairing", { sessionId, accept }),
  unpairDevice: (deviceId: string) =>
    invoke<void>("unpair_device", { deviceId }),
  setTrustLevel: (deviceId: string, level: TrustLevel) =>
    invoke<void>("set_trust_level", { deviceId, level }),

  sendFiles: (deviceId: string, paths: string[]) =>
    invoke<string>("send_files", { deviceId, paths }),
  acceptTransfer: (taskId: string, accept: boolean, saveDir?: string) =>
    invoke<void>("accept_transfer", { taskId, accept, saveDir: saveDir ?? null }),
  cancelTransfer: (taskId: string) =>
    invoke<void>("cancel_transfer", { taskId }),
  listTransfers: (limit: number, offset: number) =>
    invoke<TransferTask[]>("list_transfers", { limit, offset }),

  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) =>
    invoke<void>("update_settings", { settings }),

  listSyncScopes: () => invoke<SyncScope[]>("list_sync_scopes"),
  addSyncScope: (path: string) => invoke<SyncScope>("add_sync_scope", { path }),
  removeSyncScope: (id: string) => invoke<void>("remove_sync_scope", { id }),
  listSyncFiles: () => invoke<SyncFileEntry[]>("list_sync_files"),
  rescanSync: () => invoke<void>("rescan_sync"),
  listUnifiedFiles: () => invoke<UnifiedFile[]>("list_unified_files"),
  refreshRemoteIndex: () => invoke<void>("refresh_remote_index"),
  fetchFile: (relPath: string, hash: string | null) =>
    invoke<string>("fetch_file", { relPath, hash }),
  listConflicts: () => invoke<SyncConflict[]>("list_conflicts"),

  createShare: (relPath: string, expiresAt: number | null) =>
    invoke<Share>("create_share", { relPath, expiresAt }),
  listShares: () => invoke<Share[]>("list_shares"),
  revokeShare: (id: string) => invoke<void>("revoke_share", { id }),
  listShareAccess: (shareId: string) =>
    invoke<ShareAccess[]>("list_share_access", { shareId }),
  openShare: (link: string) => invoke<string>("open_share", { link }),

  addDownload: (url: string) => invoke<string>("add_download", { url }),
  pauseDownload: (taskId: string) =>
    invoke<void>("pause_download", { taskId }),
  resumeDownload: (taskId: string) =>
    invoke<void>("resume_download", { taskId }),
  cancelDownload: (taskId: string) =>
    invoke<void>("cancel_download", { taskId }),
  listDownloads: () => invoke<DownloadTask[]>("list_downloads"),
  pauseAllDownloads: () => invoke<number>("pause_all_downloads"),
  resumeAllDownloads: () => invoke<number>("resume_all_downloads"),
  clearCompletedDownloads: () => invoke<number>("clear_completed_downloads"),

  listArchiveRules: () => invoke<ArchiveRule[]>("list_archive_rules"),
  saveArchiveRule: (rule: ArchiveRule) =>
    invoke<ArchiveRule>("save_archive_rule", { rule }),
  deleteArchiveRule: (id: string) =>
    invoke<void>("delete_archive_rule", { id }),
  listArchiveEntries: () => invoke<ArchiveEntry[]>("list_archive_entries"),
  archiveFiles: (paths: string[], ruleId?: string, targetDir?: string) =>
    invoke<string[]>("archive_files", {
      paths,
      ruleId: ruleId ?? null,
      targetDir: targetDir ?? null,
    }),
  undoArchive: (logId: number) => invoke<void>("undo_archive", { logId }),
  listArchiveLog: () => invoke<ArchiveLogEntry[]>("list_archive_log"),

  listLocalModels: () => invoke<LocalModel[]>("list_local_models"),
  getAiStatus: () => invoke<AiStatus>("get_ai_status"),

  startSuggest: (paths: string[]) =>
    invoke<void>("start_suggest", { paths }),
  listSuggestions: () => invoke<Suggestion[]>("list_suggestions"),
  resolveSuggestion: (id: string, adopt: boolean, targetDir?: string) =>
    invoke<string | null>("resolve_suggestion", {
      id,
      adopt,
      targetDir: targetDir ?? null,
    }),

  kbAddSource: (path: string) => invoke<KbSource>("kb_add_source", { path }),
  kbRemoveSource: (id: string) => invoke<void>("kb_remove_source", { id }),
  kbListSources: () => invoke<KbSourceSummary[]>("kb_list_sources"),
  kbReindex: (sourceId: string) => invoke<void>("kb_reindex", { sourceId }),
  kbAsk: (question: string) => invoke<string>("kb_ask", { question }),
};

/** 把任意 reject 值收敛为 CommandError（兜底未知错误）。 */
export function asCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as CommandError;
  }
  return { code: "unknown", message: String(e) };
}
