// Tauri Command 的类型化封装（API_DESIGN.md §9.1）。
// 失败时 invoke 会 reject 一个 CommandError（{ code, message }）。
//
// 连接核心（设备 / 配对 / 信任 / 传输 / 同步 / 分享 / 设置）走各自的类型化 Command；
// 插件能力（下载 / 归档与 AI / 知识库）统一走 `plugin_invoke`——它们此前各占一个
// Command，加起来 27 个，是 60 个里的将近一半（R1，ARCHITECTURE.md 原则 3）。
//
// 下面这些函数的**签名一个都没变**，换的只是底层通道，所以 stores 与页面不用动。

import { invoke } from "@tauri-apps/api/core";
import type {
  AiStatus,
  ArchiveEntry,
  ArchiveLogEntry,
  ArchiveRule,
  CommandError,
  DeviceInfo,
  DownloadOptions,
  DownloadTask,
  KbSource,
  KbSourceSummary,
  LocalModel,
  LocalServerStatus,
  PendingIntroduction,
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

/// 转给某个插件。`payload` 里的键是**下划线风格**——它整个作为不透明 JSON 传给
/// 插件，由插件用 serde 反序列化，不经过 Tauri 的 camelCase→snake_case 转换。
function plugin<T>(
  id: string,
  method: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  return invoke<T>("plugin_invoke", { plugin: id, method, payload });
}

/// 本次构建装了哪些插件（id / 名字 / 设置项 schema）。
export interface PluginInfo {
  id: string;
  displayName: string;
  settingsSchema: unknown;
}

export const api = {
  pluginManifest: () => invoke<PluginInfo[]>("plugin_manifest"),

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

  // —— 信任传递 / 引荐（TRUST_DESIGN.md §5，里程碑 R2）——
  listPendingIntroductions: () =>
    invoke<PendingIntroduction[]>("list_pending_introductions"),
  confirmIntroduction: (deviceId: string) =>
    invoke<void>("confirm_introduction", { deviceId }),
  dismissIntroduction: (deviceId: string) =>
    invoke<void>("dismiss_introduction", { deviceId }),
  refreshIntroductions: () => invoke<void>("refresh_introductions"),

  // —— 内置服务器（TRUST_DESIGN.md §6.3，里程碑 R4）——
  localServerStatus: () => invoke<LocalServerStatus>("local_server_status"),

  sendFiles: (deviceId: string, paths: string[]) =>
    invoke<string>("send_files", { deviceId, paths }),
  acceptTransfer: (taskId: string, accept: boolean, saveDir?: string) =>
    invoke<void>("accept_transfer", { taskId, accept, saveDir: saveDir ?? null }),
  pauseTransfer: (taskId: string) =>
    invoke<void>("pause_transfer", { taskId }),
  resumeTransfer: (taskId: string) =>
    invoke<void>("resume_transfer", { taskId }),
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

  addDownload: (url: string, options?: DownloadOptions) =>
    plugin<string>("download", "add", { url, options: options ?? null }),
  addTorrentFile: (path: string, options?: DownloadOptions) =>
    plugin<string>("download", "add_torrent_file", {
      path,
      options: options ?? null,
    }),
  pauseDownload: (taskId: string) =>
    plugin<void>("download", "pause", { id: taskId }),
  resumeDownload: (taskId: string) =>
    plugin<void>("download", "resume", { id: taskId }),
  cancelDownload: (taskId: string, deleteLocal = false) =>
    plugin<void>("download", "cancel", { id: taskId, delete_local: deleteLocal }),
  retryDownload: (taskId: string) =>
    plugin<string>("download", "retry", { id: taskId }),
  listDownloads: () => plugin<DownloadTask[]>("download", "list"),
  pauseAllDownloads: () => plugin<number>("download", "pause_all"),
  resumeAllDownloads: () => plugin<number>("download", "resume_all"),
  clearCompletedDownloads: () => plugin<number>("download", "clear_completed"),

  listArchiveRules: () => plugin<ArchiveRule[]>("archive", "list_rules"),
  saveArchiveRule: (rule: ArchiveRule) =>
    plugin<ArchiveRule>("archive", "save_rule", { rule }),
  deleteArchiveRule: (id: string) =>
    plugin<void>("archive", "delete_rule", { id }),
  listArchiveEntries: () => plugin<ArchiveEntry[]>("archive", "list_entries"),
  archiveFiles: (paths: string[], ruleId?: string, targetDir?: string) =>
    plugin<string[]>("archive", "archive_files", {
      paths,
      rule_id: ruleId ?? null,
      target_dir: targetDir ?? null,
    }),
  undoArchive: (logId: number) =>
    plugin<void>("archive", "undo", { log_id: logId }),
  listArchiveLog: () => plugin<ArchiveLogEntry[]>("archive", "list_log"),

  listLocalModels: () => plugin<LocalModel[]>("archive", "list_local_models"),
  getAiStatus: () => plugin<AiStatus>("archive", "ai_status"),

  startSuggest: (paths: string[]) =>
    plugin<void>("archive", "start_suggest", { paths }),
  listSuggestions: () => plugin<Suggestion[]>("archive", "list_suggestions"),
  resolveSuggestion: (id: string, adopt: boolean, targetDir?: string) =>
    plugin<string | null>("archive", "resolve_suggestion", {
      id,
      adopt,
      target_dir: targetDir ?? null,
    }),

  kbAddSource: (path: string) => plugin<KbSource>("archive", "kb_add_source", { path }),
  kbRemoveSource: (id: string) => plugin<void>("archive", "kb_remove_source", { id }),
  kbListSources: () => plugin<KbSourceSummary[]>("archive", "kb_list_sources"),
  kbReindex: (sourceId: string) =>
    plugin<void>("archive", "kb_reindex", { id: sourceId }),
  kbAsk: (question: string) => plugin<string>("archive", "kb_ask", { question }),
};

/** 把任意 reject 值收敛为 CommandError（兜底未知错误）。 */
export function asCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as CommandError;
  }
  return { code: "unknown", message: String(e) };
}
