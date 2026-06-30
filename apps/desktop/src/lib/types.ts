// 前后端共享类型的 TS 镜像（与 aa4c-types 一致，JSON camelCase）。
// 契约见 API_DESIGN.md §3 / §9。

export type Platform =
  | "windows"
  | "macos"
  | "linux"
  | "android"
  | "ios"
  | "server";

export type Direction = "send" | "recv";

export type TransferStatus =
  | "waiting_accept"
  | "transferring"
  | "done"
  | "failed"
  | "cancelled"
  | "rejected";

export type FileStatus = "pending" | "transferring" | "done" | "failed";

/** 信任分级：full=我的设备（参与同步）/ friend=朋友（仅收发）。 */
export type TrustLevel = "full" | "friend";

export interface DeviceInfo {
  id: string;
  name: string;
  platform: Platform;
  version: string;
  /** "ip:port"，可能为 null（离线/未解析）。 */
  addr: string | null;
  online: boolean;
  trusted: boolean;
  /** 信任分级；未配对（仅发现）的设备为 null。 */
  trustLevel: TrustLevel | null;
}

export interface TransferFile {
  relPath: string;
  size: number;
  hash: string | null;
  status: FileStatus;
}

export interface TransferTask {
  id: string;
  direction: Direction;
  peer: string;
  files: TransferFile[];
  status: TransferStatus;
  totalBytes: number;
  transferredBytes: number;
  /** unix 毫秒。 */
  createdAt: number;
  error: string | null;
}

export interface Settings {
  deviceName: string;
  saveDir: string;
  autoAcceptFromTrusted: boolean;
  listenPort: number;
}

/** 共享范围种类：用户选的同步文件夹，或固定的「收到的」(自动维护)。 */
export type ScopeKind = "folder" | "inbox";

export interface SyncScope {
  id: string;
  kind: ScopeKind;
  localPath: string;
  /** unix 毫秒。 */
  createdAt: number;
}

/** 本机文件索引条目（V0.2 里程碑 2：跨设备黄/红状态留待后续里程碑）。 */
export interface SyncFileEntry {
  scopeId: string;
  relPath: string;
  size: number;
  mtime: number;
  hash: string | null;
  presentLocal: boolean;
}

/** Command 失败时后端返回的形状（API_DESIGN §9.1）。 */
export interface CommandError {
  code: string;
  message: string;
}

// —— 事件 payload（API_DESIGN §9.2，扁平 camelCase）——

export interface DeviceLostPayload {
  id: string;
}
export interface PairingRequestPayload {
  sessionId: string;
  peer: DeviceInfo;
}
export interface PairingPinPayload {
  sessionId: string;
  pin: string;
}
export interface PairingResultPayload {
  sessionId: string;
  peer: string;
  success: boolean;
}
export interface TransferRequestPayload {
  task: TransferTask;
}
export interface TransferProgressPayload {
  taskId: string;
  transferredBytes: number;
  totalBytes: number;
  speedBps: number;
  currentFile: string;
}
export interface TransferDonePayload {
  taskId: string;
}
export interface TransferFailedPayload {
  taskId: string;
  error: string;
}
