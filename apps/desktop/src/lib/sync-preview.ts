// 同步页预览数据。
//
// 跨设备文件索引（绿/黄/红）的后端尚未实现（设计见 SYNC_DESIGN.md），
// 这里用示例数据先把界面做成设计稿，便于评审与打磨；V0.2 接真实索引时替换。

/** 文件可获取状态：local=本地有(绿) / online=在线设备有可下载(黄) / offline=仅离线设备有(红)。 */
export type SyncStatus = "local" | "online" | "offline";

export interface SyncEntry {
  /** 文件名（展示用，多版本已加序号）。 */
  name: string;
  size: number;
  status: SyncStatus;
  /** 持有 / 来源设备名。 */
  owner: string;
}

export interface SyncGroup {
  /** 分组标题（共享范围 / 目录 / Inbox 分组）。 */
  title: string;
  entries: SyncEntry[];
}

/** 示例：覆盖三种状态 + Inbox 分组 + 同名多版本（加序号）。 */
export const SAMPLE_GROUPS: SyncGroup[] = [
  {
    title: "照片库",
    entries: [
      { name: "IMG_2024.jpg", size: 3_900_000, status: "local", owner: "这台设备" },
      { name: "IMG_2025.jpg", size: 4_200_000, status: "online", owner: "我的 MacBook" },
      { name: "旅行_2026.mp4", size: 1_280_000_000, status: "offline", owner: "家里 NAS" },
    ],
  },
  {
    title: "收到的 · 来自 客厅电脑 · 今天",
    entries: [
      { name: "合同.pdf", size: 820_000, status: "online", owner: "客厅电脑" },
      { name: "合同 (2).pdf", size: 845_000, status: "online", owner: "客厅电脑" },
    ],
  },
  {
    title: "工作",
    entries: [
      { name: "设计稿.sketch", size: 56_000_000, status: "local", owner: "这台设备" },
      { name: "README.md", size: 12_000, status: "local", owner: "这台设备" },
      { name: "模型_Qwen3.gguf", size: 8_300_000_000, status: "offline", owner: "工作站" },
    ],
  },
];

/** 状态 → 颜色点的图例文案（人话，UI_DESIGN_SPEC §7）。 */
export const STATUS_LEGEND: { status: SyncStatus; label: string }[] = [
  { status: "local", label: "本地有" },
  { status: "online", label: "可下载" },
  { status: "offline", label: "设备离线" },
];
