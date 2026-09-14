// 展示层格式化与文案（UI_DESIGN_SPEC.md §6 / §7：说人话、零术语）。

import type {
  ConnectionVia,
  ReachFailure,
  ReachState,
  DownloadStatus,
  Platform,
  TransferStatus,
} from "./types";

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
    case "paused":
      return "已暂停";
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

/** 下载任务卡片标题：HTTP 直链走 `baseName`；magnet 链接不是路径，取 `dn=`
 *  显示名参数，没有的话退化成截短的 infohash（同 D1 baseName 惯例，不出现
 *  "magnet:"/"xt="/"btih" 这类技术词）。 */
export function taskTitle(url: string, id: string): string {
  if (!url.startsWith("magnet:")) return baseName(url);
  const dn = new URLSearchParams(url.slice(url.indexOf("?") + 1)).get("dn");
  if (dn) return dn;
  return `BT 任务 ${id.slice(0, 8)}`;
}

/** unix 毫秒 → "14:32" 时刻。 */
export function timeText(createdAtMs: number): string {
  const d = new Date(createdAtMs);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** 连接档位 → 人话（AGENTS.md UI 规则：不出现 NAT / 打洞 / STUN 这类技术词）。
 *
 * 文案收在这一处，传输卡片与首页的设备状态图共用——两边说法不一致会让人以为
 * 是两种不同的东西。`undefined` 返回空串：只有发起方收得到档位，接收一方本地
 * 永远是空，**不确定就不显示**，比猜一个诚实。 */
export function connectionViaText(via: ConnectionVia | undefined): string {
  switch (via) {
    case "lan":
      return "局域网直连";
    case "public_v4":
      return "公网直连";
    case "public_v6":
      return "公网直连（IPv6）";
    case "punch":
      // 打洞成功后就是真直连，用户不需要关心过程（CONNECT_DESIGN.md §10）。
      return "直连";
    case "relay":
      return "中继（较慢）";
    default:
      return "";
  }
}

/** 档位的提示色：中继要自建服务器、而且慢，值得单独标出来。 */
export function connectionViaTone(via: ConnectionVia | undefined): "ok" | "warn" | "" {
  if (!via) return "";
  return via === "relay" ? "warn" : "ok";
}

/** 可达状态 → 人话。`unknown` 说「正在检查」而不是「离线」——
 * 我们确实还不知道，说成离线是在编造事实。 */
export function reachStateText(state: ReachState): string {
  switch (state) {
    case "reachable":
      return "已连接";
    case "unreachable":
      return "连不上";
    default:
      return "正在检查…";
  }
}

/** 连不上的原因 → 人话 + **下一步**（UI_DESIGN_SPEC §6：光说坏了没用，要说怎么办）。
 * 不出现 NAT / STUN / mDNS 这类词（AGENTS.md UI 规则）。 */
export function reachFailureText(reason: ReachFailure | undefined): string {
  switch (reason) {
    case "not_on_lan_and_remote_off":
      return "不在同一个网络，而且远程连接没开。把两台设备连到同一个 WiFi，或在设置里打开远程连接。";
    case "peer_refused":
      return "对方拒绝了连接，多半是版本太旧。确认两台设备都升级到最新版。";
    case "unreachable":
      return "试过了连不上。确认对方开着 AA连接，并检查防火墙有没有放行。";
    default:
      return "";
  }
}
