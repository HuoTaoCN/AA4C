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
  /** 下载目录（默认系统下载目录），必须在 saveDir 子树之外（里程碑 D1）。 */
  downloadDir: string;
  /** 下载限速（KB/s），null = 不限速。重启引擎生效（里程碑 D3）。 */
  downloadSpeedLimitKbps: number | null;
  /** 并发下载数，null = 引擎默认。重启引擎生效（里程碑 D3）。 */
  downloadConcurrency: number | null;
  /** BT 分享率上限，null = 不限（里程碑 D3）。 */
  btRatioLimit: number | null;
  /** BT 空闲做种超时（分钟），null = 不限——多久没有上传活动就停止做种，
   * 不是"总做种时长"（里程碑 D3）。 */
  btIdleSeedingLimitMinutes: number | null;
  /** 归档根目录，必须在 saveDir/downloadDir 子树之外（里程碑 AI1）。 */
  archiveRoot: string;
  /** 自动归档总闸（下载完成后跑规则引擎），默认开启；真正的保守闸门在每条规则
   * 各自的 enabled（默认停用），见 ARCHIVE_DESIGN.md §2.3（里程碑 AI1）。 */
  archiveAutoEnabled: boolean;
}

/** 一次连接实际走的档位（里程碑 C4 连接质量 + C5 打洞，见 CONNECT_DESIGN.md §2）。
 * `punch`（打洞后升级成的直连）在 UI 上并入「直连」显示，不单独暴露成第三个词。 */
export type ConnectionVia = "direct" | "punch" | "relay";

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

/** 一条分享记录（里程碑 C6，CONNECT_DESIGN.md §7/§8）。 */
export interface Share {
  id: string;
  token: string;
  relPath: string;
  /** 目前恒为 "read"。 */
  permission: string;
  /** unix 毫秒；null = 长期有效。 */
  expiresAt: number | null;
  /** "open" | "revoked"。 */
  status: string;
  createdAt: number;
  /** 完整可分享链接（`aa4c://share/...`）。 */
  link: string;
}

/** 一条分享访问记录（可选功能）。 */
export interface ShareAccess {
  id: number;
  shareId: string;
  peerId: string | null;
  action: string;
  at: number;
}

/** 'bt' 是 D2（Transmission/Magnet）。 */
export type DownloadKind = "http" | "bt";

export type DownloadStatus =
  | "active"
  | "waiting"
  | "paused"
  | "error"
  | "complete"
  | "removed";

/** 一条下载任务（里程碑 D1，DOWNLOAD_DESIGN.md §4）。 */
export interface DownloadTask {
  id: string;
  kind: DownloadKind;
  url: string;
  savePath: string | null;
  status: DownloadStatus;
  totalBytes: number;
  downloadedBytes: number;
  error: string | null;
  /** unix 毫秒。 */
  createdAt: number;
}

/** 内置类别（不可增删，标签才是用户的自由维度，见 ARCHIVE_DESIGN.md §2.1）。 */
export type ArchiveCategory =
  | "model"
  | "image"
  | "video"
  | "audio"
  | "document"
  | "ebook"
  | "archive"
  | "installer"
  | "code"
  | "subtitle"
  | "other";

/** 规则匹配条件；categories 为空数组视为"任意类别都匹配"（里程碑 AI1）。 */
export interface ArchiveMatch {
  categories: ArchiveCategory[];
  extensions: string[] | null;
  glob: string | null;
  minSize: number | null;
  maxSize: number | null;
}

/** 目标目录模板占位符：{类别} {年} {月} {扩展名} {模型.架构} {模型.名称} {模型.量化}，
 * 缺值时用"未知"，绝不失败中断（ARCHIVE_DESIGN.md §2.3）。 */
export interface ArchiveAction {
  targetTemplate: string;
  tags: string[];
}

/** 一条归档规则（里程碑 AI1，ARCHIVE_DESIGN.md §2.3）。新建时 id 传空串，
 * 后端会生成 uuid 再返回完整规则。 */
export interface ArchiveRule {
  id: string;
  name: string;
  enabled: boolean;
  position: number;
  matcher: ArchiveMatch;
  action: ArchiveAction;
  /** unix 毫秒。 */
  createdAt: number;
  updatedAt: number;
}

/** GGUF 头解析出的模型元数据，仅"模型"类别的 ArchiveEntry 非 null
 * （ARCHIVE_DESIGN.md §2.2）。 */
export interface ModelMeta {
  architecture: string | null;
  name: string | null;
  sizeLabel: string | null;
  fileType: string | null;
  contextLength: number | null;
}

/** 一条被归档引擎移动/纳管的文件记录。 */
export interface ArchiveEntry {
  id: string;
  currentPath: string;
  category: ArchiveCategory;
  size: number;
  modelMeta: ModelMeta | null;
  createdAt: number;
  updatedAt: number;
}

/** 一条移动历史（撤销要靠 id，ARCHIVE_DESIGN.md §2.4）。ruleId 为 null 代表手动归档。 */
export interface ArchiveLogEntry {
  id: number;
  entryId: string;
  fromPath: string;
  toPath: string;
  ruleId: string | null;
  /** unix 毫秒。 */
  at: number;
  undone: boolean;
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
export interface DownloadProgressPayload {
  taskId: string;
  downloadedBytes: number;
  totalBytes: number;
  speedBps: number;
  /** D2（BT）专属，HTTP 任务不出现这三个字段（不是 null，是整个 key 不存在）。 */
  seeders?: number;
  peers?: number;
  ratio?: number;
}
export interface DownloadDonePayload {
  taskId: string;
  savePath: string;
}
export interface DownloadFailedPayload {
  taskId: string;
  error: string;
}
/** ruleId 为 null 代表手动归档（里程碑 AI1）。 */
export interface ArchiveAppliedPayload {
  entryId: string;
  fromPath: string;
  toPath: string;
  ruleId?: string;
}
