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
  /** 自建 aa4c-server 地址（`aa4c://host:port#指纹`），未配置为 null（里程碑 C2/C4）。 */
  serverUrl: string | null;
  /** 远程连接总开关，默认关闭（里程碑 C2/C4）。 */
  enableRemote: boolean;
}

/** 一次连接实际走的档位（里程碑 C4 连接质量，见 CONNECT_DESIGN.md §2）。 */
export type ConnectionVia = "direct" | "relay";

/** 共享范围种类：用户选的同步文件夹，或固定的「收到的」(自动维护)。 */
export type ScopeKind = "folder" | "inbox";

export interface SyncScope {
  id: string;
  kind: ScopeKind;
  localPath: string;
  /** unix 毫秒。 */
  createdAt: number;
}

/** 本机文件索引条目（本机扫描出的原始条目，调试/兼容用）。 */
export interface SyncFileEntry {
  scopeId: string;
  relPath: string;
  size: number;
  mtime: number;
  hash: string | null;
  presentLocal: boolean;
}

/** 文件可获取状态（SYNC_DESIGN §4）：🟢 本地有 / 🟡 可下载 / 🔴 设备离线。 */
export type SyncStatusCode = "local" | "online" | "offline";

/** 统一文件视图条目（里程碑 3 + 5）：本机 + 跨设备索引归并后的结果。 */
export interface UnifiedFile {
  /** 限定展示路径，`/` 分隔；顶层段是来源分组。冲突时按序号区分（`报告 (2).pdf`）。 */
  relPath: string;
  /** 限定基准路径（未加序号，对端认得的真实路径）；拉取按 basePath + hash 定位。 */
  basePath: string;
  size: number;
  hash: string | null;
  status: SyncStatusCode;
  /** 持有该文件的设备名（本机用「这台设备」）。 */
  holders: string[];
  /** 是否为冲突版本之一（同一 basePath 有多个不同 hash）。 */
  conflict: boolean;
}

/** 冲突记录（里程碑 5）：同一基准路径存在多个不同 hash 的版本。 */
export interface SyncConflict {
  relPath: string;
  hash: string;
  status: string;
  createdAt: number;
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
export interface TransferConnectedPayload {
  taskId: string;
  via: ConnectionVia;
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
