// 同步页预览数据（树形）。
//
// 跨设备文件索引（绿/黄/红）的后端尚未实现（设计见 SYNC_DESIGN.md），
// 这里用示例数据先把界面做成设计稿，便于评审与打磨；V0.2 接真实索引时替换。

/** 文件可获取状态：local=本地有(绿) / online=在线设备有可下载(黄) / offline=仅离线设备有(红)。 */
export type SyncStatus = "local" | "online" | "offline";

export interface SyncFile {
  kind: "file";
  name: string;
  size: number;
  status: SyncStatus;
  /** 持有该文件的设备名；online 多设备时可并行拉取（更快）。 */
  owners: string[];
}

export interface SyncDir {
  kind: "dir";
  name: string;
  children: SyncNode[];
}

export type SyncNode = SyncFile | SyncDir;

/** 状态 → 文字标签（人话，UI_DESIGN_SPEC §7）。 */
export function statusLabel(s: SyncStatus): string {
  return s === "local" ? "本地有" : s === "online" ? "可下载" : "设备离线";
}

export const STATUS_LEGEND: { status: SyncStatus; label: string }[] = [
  { status: "local", label: "本地有" },
  { status: "online", label: "可下载" },
  { status: "offline", label: "设备离线" },
];

/** 示例树：覆盖三种状态 + 嵌套目录 + Inbox 分组 + 多版本(加序号) + 多设备持有。 */
export const SAMPLE_TREE: SyncNode[] = [
  {
    kind: "dir",
    name: "照片库",
    children: [
      {
        kind: "dir",
        name: "2024",
        children: [
          { kind: "file", name: "IMG_2024.jpg", size: 3_900_000, status: "local", owners: ["这台设备"] },
          {
            kind: "file",
            name: "IMG_2025.jpg",
            size: 4_200_000,
            status: "online",
            owners: ["我的 MacBook", "工作站"],
          },
        ],
      },
      { kind: "file", name: "旅行_2026.mp4", size: 1_280_000_000, status: "offline", owners: ["家里 NAS"] },
    ],
  },
  {
    kind: "dir",
    name: "收到的 · 来自 客厅电脑 · 今天",
    children: [
      { kind: "file", name: "合同.pdf", size: 820_000, status: "online", owners: ["客厅电脑"] },
      { kind: "file", name: "合同 (2).pdf", size: 845_000, status: "online", owners: ["客厅电脑"] },
    ],
  },
  {
    kind: "dir",
    name: "工作",
    children: [
      { kind: "file", name: "设计稿.sketch", size: 56_000_000, status: "local", owners: ["这台设备"] },
      { kind: "file", name: "README.md", size: 12_000, status: "local", owners: ["这台设备"] },
      {
        kind: "dir",
        name: "模型",
        children: [
          { kind: "file", name: "模型_Qwen3.gguf", size: 8_300_000_000, status: "offline", owners: ["工作站"] },
        ],
      },
    ],
  },
];

/** 按谓词裁剪树：保留命中的文件，以及含命中后代的目录。 */
export function pruneTree(nodes: SyncNode[], keep: (f: SyncFile) => boolean): SyncNode[] {
  const out: SyncNode[] = [];
  for (const n of nodes) {
    if (n.kind === "file") {
      if (keep(n)) out.push(n);
    } else {
      const children = pruneTree(n.children, keep);
      if (children.length) out.push({ ...n, children });
    }
  }
  return out;
}
