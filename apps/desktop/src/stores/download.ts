// 下载中心 store（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  DownloadDonePayload,
  DownloadFailedPayload,
  DownloadProgressPayload,
  DownloadTask,
} from "../lib/types";

/** 进行中任务：在 DownloadTask 上叠加实时速度（不落库，同 transfer store 先例）。 */
export interface LiveDownloadTask extends DownloadTask {
  speedBps: number;
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
     *  的写入完全体现在下一次 tellActive 快照里，刷新一次拿最新状态）。 */
    async add(url: string): Promise<string> {
      const id = await api.addDownload(url);
      await this.load();
      return id;
    },

    async pause(taskId: string) {
      await api.pauseDownload(taskId);
    },
    async resume(taskId: string) {
      await api.resumeDownload(taskId);
    },
    async cancel(taskId: string) {
      await api.cancelDownload(taskId);
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
