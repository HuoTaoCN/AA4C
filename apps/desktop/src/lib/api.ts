// 11 个 Tauri Command 的类型化封装（API_DESIGN.md §9.1）。
// 失败时 invoke 会 reject 一个 CommandError（{ code, message }）。

import { invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  DeviceInfo,
  Settings,
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
  fetchFile: (relPath: string) => invoke<string>("fetch_file", { relPath }),
};

/** 把任意 reject 值收敛为 CommandError（兜底未知错误）。 */
export function asCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as CommandError;
  }
  return { code: "unknown", message: String(e) };
}
