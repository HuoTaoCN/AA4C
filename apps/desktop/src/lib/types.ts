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

export interface DeviceInfo {
  id: string;
  name: string;
  platform: Platform;
  version: string;
  /** "ip:port"，可能为 null（离线/未解析）。 */
  addr: string | null;
  online: boolean;
  trusted: boolean;
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
