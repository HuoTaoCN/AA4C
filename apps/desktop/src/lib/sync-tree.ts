// 同步页的树形视图：把后端的统一文件索引组装成目录树。
//
// V0.2 里程碑 3：条目来自「本机索引 + 跨设备远端索引」的归并（后端 list_unified_files），
// 每条自带 🟢 本地有 / 🟡 可下载 / 🔴 设备离线 状态与持有设备名。条目的 relPath
// 已是限定路径（顶层段=来源分组：「收到的」或共享文件夹名），按 "/" 拆分即得目录树。

import type { UnifiedFile } from "./types";

/** 文件可获取状态：local=本地有(绿) / online=在线设备有可下载(黄) / offline=仅离线设备有(红)。 */
export type SyncStatus = "local" | "online" | "offline";

export interface SyncFile {
  kind: "file";
  name: string;
  /** 完整限定路径（顶层段=来源分组），按需拉取时回传给后端。 */
  relPath: string;
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

function insert(dir: SyncDir, segments: string[], file: UnifiedFile) {
  const [head, ...rest] = segments;
  if (rest.length === 0) {
    dir.children.push({
      kind: "file",
      name: head,
      relPath: file.relPath,
      size: file.size,
      status: file.status,
      owners: file.holders,
    });
    return;
  }
  let child = dir.children.find(
    (c): c is SyncDir => c.kind === "dir" && c.name === head,
  );
  if (!child) {
    child = { kind: "dir", name: head, children: [] };
    dir.children.push(child);
  }
  insert(child, rest, file);
}

function sortDir(dir: SyncDir) {
  dir.children.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    return a.name.localeCompare(b.name, "zh");
  });
  for (const c of dir.children) if (c.kind === "dir") sortDir(c);
}

/** 把统一文件索引（限定路径，顶层段=来源分组）组装成目录树；空分组自然不出现。 */
export function buildTree(files: UnifiedFile[]): SyncNode[] {
  const roots = new Map<string, SyncDir>();
  for (const file of files) {
    const segments = file.relPath.split("/");
    const group = segments[0];
    if (!group) continue;
    let root = roots.get(group);
    if (!root) {
      root = { kind: "dir", name: group, children: [] };
      roots.set(group, root);
    }
    insert(root, segments.slice(1), file);
  }
  const tree = [...roots.values()];
  tree.sort((a, b) => a.name.localeCompare(b.name, "zh"));
  tree.forEach(sortDir);
  return tree;
}

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
