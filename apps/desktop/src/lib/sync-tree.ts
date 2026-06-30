// 同步页的树形视图：把后端的扁平文件索引（按共享范围）组装成目录树。
//
// V0.2 里程碑 2：索引只来自本机扫描，所以每个条目都是「本地有」(绿)。
// 黄(可下载)/红(设备离线) 要等里程碑 3 的跨设备索引交换才会出现，
// 这里的类型已经把三态留好，方便后续直接接上。

import type { SyncFileEntry, SyncScope } from "./types";

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

/** 范围在树里的显示名：Inbox 固定叫「收到的」，文件夹取路径末段。 */
function scopeName(scope: SyncScope): string {
  if (scope.kind === "inbox") return "收到的";
  const parts = scope.localPath.replace(/[/\\]+$/, "").split(/[/\\]/);
  return parts[parts.length - 1] || scope.localPath;
}

function insert(dir: SyncDir, segments: string[], file: SyncFileEntry) {
  const [head, ...rest] = segments;
  if (rest.length === 0) {
    dir.children.push({
      kind: "file",
      name: head,
      size: file.size,
      // V0.2 里程碑 2：本机索引里的条目恒为本地有，里程碑 3 接入跨设备状态后这里会按需判定
      status: "local",
      owners: ["这台设备"],
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

/** 把（范围 + 扁平文件索引）组装成目录树；每个范围是一个顶层目录，空范围不显示。 */
export function buildTree(scopes: SyncScope[], files: SyncFileEntry[]): SyncNode[] {
  const roots = new Map<string, SyncDir>();
  for (const scope of scopes) {
    roots.set(scope.id, { kind: "dir", name: scopeName(scope), children: [] });
  }
  for (const file of files) {
    const root = roots.get(file.scopeId);
    if (!root) continue;
    insert(root, file.relPath.split("/"), file);
  }
  const tree = [...roots.values()].filter((r) => r.children.length > 0);
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
