// 展示层格式化与文案（UI_DESIGN_SPEC.md §6 / §7：说人话、零术语）。

import type { DownloadStatus, Platform, TransferStatus } from "./types";

/** 字节数 → 人类可读（1.2 GB / 42 MB / 800 KB）。 */
export function humanBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[i]}`;
}

/** 速度 → "42 MB/s"。 */
export function humanSpeed(bytesPerSec: number): string {
  return `${humanBytes(bytesPerSec)}/s`;
}

/** 剩余时间 → "剩约 12 秒" / "剩约 3 分钟"。 */
export function etaText(remainingBytes: number, speedBps: number): string {
  if (speedBps <= 0) return "";
  const secs = Math.ceil(remainingBytes / speedBps);
  if (secs < 60) return `剩约 ${secs} 秒`;
  return `剩约 ${Math.ceil(secs / 60)} 分钟`;
}

/** 平台图标（emoji 占位，V0.2 换线性图标）。 */
export function platformIcon(platform: Platform): string {
  switch (platform) {
    case "android":
    case "ios":
      return "📱";
    case "server":
      return "🗄";
    case "macos":
      return "🖥";
    default:
      return "💻";
  }
}

/** 错误码 → 人话文案 + 下一步（UI_DESIGN_SPEC §6）。 */
export function errorText(code: string): string {
  switch (code) {
    case "not_paired":
      return "还没有和这台设备配对，先配对一下吧";
    case "network":
    case "device_not_found":
      return "连不上对方设备，请确认两台设备在同一个 WiFi";
    case "transfer_rejected":
      return "对方拒绝了这次传输";
    case "pairing_rejected":
      return "对方拒绝了配对";
    case "pin_mismatch":
      return "确认码不一致，配对未完成";
    case "hash_mismatch":
      return "传输出了点问题，文件不完整，请重新发送";
    case "cancelled":
      return "已取消";
    case "io":
      return "空间不够了，清理一下磁盘或换个保存位置";
    case "unavailable":
      return "下载功能当前不可用，请重启应用后重试";
    default:
      return "出了点小问题，请重试";
  }
}

/** 任务状态 → 中文短语（人话，不出现技术词）。 */
export function statusText(status: TransferStatus): string {
  switch (status) {
    case "waiting_accept":
      return "等待对方确认";
    case "transferring":
      return "正在传输";
    case "done":
      return "已完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
    case "rejected":
      return "被拒绝";
  }
}

/** 下载任务状态 → 中文短语（人话，不出现 aria2/GID/RPC 等技术词）。 */
export function downloadStatusText(status: DownloadStatus): string {
  switch (status) {
    case "active":
      return "下载中";
    case "waiting":
      return "排队中";
    case "paused":
      return "已暂停";
    case "error":
      return "失败";
    case "complete":
      return "已完成";
    case "removed":
      return "已取消";
  }
}

/** unix 毫秒 → 分组键（今天 / 昨天 / 更早）。 */
export function dayGroup(createdAtMs: number): "今天" | "昨天" | "更早" {
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const oneDay = 24 * 60 * 60 * 1000;
  if (createdAtMs >= startOfToday) return "今天";
  if (createdAtMs >= startOfToday - oneDay) return "昨天";
  return "更早";
}

/** 路径 → 文件名（兼容 / 与 \\ 分隔）。 */
export function baseName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

/** unix 毫秒 → "14:32" 时刻。 */
export function timeText(createdAtMs: number): string {
  const d = new Date(createdAtMs);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
