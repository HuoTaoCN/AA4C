// 下载中心 store（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  DownloadDonePayload,
  DownloadFailedPayload,
  DownloadOptions,
  DownloadProgressPayload,
  DownloadTask,
} from "../lib/types";

/** 进行中任务：在 DownloadTask 上叠加实时速度（不落库，同 transfer store 先例）。
 *  seeders/peers/ratio 是 D2（BT）专属，HTTP 任务恒为 undefined。 */
export interface LiveDownloadTask extends DownloadTask {
  speedBps: number;
  seeders?: number;
  peers?: number;
  ratio?: number;
}

interface State {
  tasks: Record<string, LiveDownloadTask>;
  loading: boolean;
}

export const useDownloadStore = defineStore("download", {
  state: (): State => ({ tasks: {}, loading: false }),

  getters: {
    list: (s): LiveDownloadTask[] =>
      Object.values(s.tasks).sort((a, b) => b.createdAt - a.createdAt),
    /** 批量操作按钮的显隐判断（D3）。 */
    hasActiveOrWaiting: (s): boolean =>
      Object.values(s.tasks).some(
        (t) => t.status === "active" || t.status === "waiting",
      ),
    hasPaused: (s): boolean =>
      Object.values(s.tasks).some((t) => t.status === "paused"),
    hasCompleted: (s): boolean =>
      Object.values(s.tasks).some((t) => t.status === "complete"),
    /** 顶部统计条（对标 FDM/Motrix 的全局速度显示）：进行中任务的速度总和。 */
    totalSpeedBps: (s): number =>
      Object.values(s.tasks)
        .filter((t) => t.status === "active")
        .reduce((sum, t) => sum + t.speedBps, 0),
    activeCount: (s): number =>
      Object.values(s.tasks).filter(
        (t) => t.status === "active" || t.status === "waiting",
      ).length,
    /** 重复链接检测（对标 FDM 的"该链接已在下载列表"提示）：只认还"在列表里"
     *  的任务——已清除（removed）的不算重复，允许用户重新添加同一个链接。 */
    findByUrl:
      (s) =>
      (url: string): LiveDownloadTask | undefined =>
        Object.values(s.tasks).find(
          (t) => t.url === url && t.status !== "removed",
        ),
  },

  actions: {
    async load() {
      this.loading = true;
      try {
        const tasks = await api.listDownloads();
        const next: Record<string, LiveDownloadTask> = {};
        for (const t of tasks) {
          next[t.id] = { ...t, speedBps: this.tasks[t.id]?.speedBps ?? 0 };
        }
        this.tasks = next;
      } finally {
        this.loading = false;
      }
    },

    /** 新建一条下载任务，立即刷新列表（新任务此时可能还没被 `add_download` 落库
     *  的写入完全体现在下一次 tellActive 快照里，刷新一次拿最新状态）。
     *  `options` 是每任务的自定义选项（保存位置/文件名/Referer/Cookie）。 */
    async add(url: string, options?: DownloadOptions): Promise<string> {
      const id = await api.addDownload(url, options);
      await this.load();
      return id;
    },

    /** 从本地 .torrent 文件新建 BT 任务。 */
    async addTorrentFile(path: string, options?: DownloadOptions): Promise<string> {
      const id = await api.addTorrentFile(path, options);
      await this.load();
      return id;
    },

    async pause(taskId: string) {
      await api.pauseDownload(taskId);
    },
    async resume(taskId: string) {
      await api.resumeDownload(taskId);
    },
    async cancel(taskId: string, deleteLocal = false) {
      await api.cancelDownload(taskId, deleteLocal);
    },
    /** 重试失败任务：HTTP 任务会产生一个新 id（aria2 不能复活旧 GID），刷新列表
     *  是唯一能让新任务出现的方式（同 `add` 的既有惯例）。 */
    async retry(taskId: string): Promise<string> {
      const id = await api.retryDownload(taskId);
      await this.load();
      return id;
    },

    /** 批量操作（D3）：返回值是实际生效的数量，调用方决定 toast 文案；三个
     *  都在操作后刷新列表（同 `add` 的既有惯例）。 */
    async pauseAll(): Promise<number> {
      const n = await api.pauseAllDownloads();
      await this.load();
      return n;
    },
    async resumeAll(): Promise<number> {
      const n = await api.resumeAllDownloads();
      await this.load();
      return n;
    },
    async clearCompleted(): Promise<number> {
      const n = await api.clearCompletedDownloads();
      await this.load();
      return n;
    },

    onProgress(p: DownloadProgressPayload) {
      const base = this.tasks[p.taskId];
      if (!base) return; // 未知任务的进度事件：等下一次 load() 兜底，不强行拼一条骨架
      this.tasks[p.taskId] = {
        ...base,
        status: "active",
        downloadedBytes: p.downloadedBytes,
        totalBytes: p.totalBytes || base.totalBytes,
        speedBps: p.speedBps,
        seeders: p.seeders,
        peers: p.peers,
        ratio: p.ratio,
      };
    },

    onDone(p: DownloadDonePayload) {
      const base = this.tasks[p.taskId];
      if (base) {
        this.tasks[p.taskId] = {
          ...base,
          status: "complete",
          savePath: p.savePath,
          speedBps: 0,
        };
      } else {
        void this.load();
      }
    },

    onFailed(p: DownloadFailedPayload) {
      const base = this.tasks[p.taskId];
      if (base) {
        this.tasks[p.taskId] = { ...base, status: "error", error: p.error, speedBps: 0 };
      } else {
        void this.load();
      }
    },
  },
});
