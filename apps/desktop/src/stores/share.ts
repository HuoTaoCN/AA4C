// 分享链接 store（CONNECT_DESIGN.md §7，里程碑 C6）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type { Share } from "../lib/types";

interface State {
  shares: Share[];
  loading: boolean;
}

export const useShareStore = defineStore("share", {
  state: (): State => ({ shares: [], loading: false }),

  actions: {
    async load() {
      this.loading = true;
      try {
        this.shares = await api.listShares();
      } finally {
        this.loading = false;
      }
    },
    /** 生成一条新分享；`expiresAt` 为 null 表示长期有效。 */
    async create(relPath: string, expiresAt: number | null): Promise<Share> {
      const share = await api.createShare(relPath, expiresAt);
      await this.load();
      return share;
    },
    async revoke(id: string) {
      await api.revokeShare(id);
      await this.load();
    },
    /** 打开一个分享链接，返回本机接收任务 id。 */
    async open(link: string): Promise<string> {
      return api.openShare(link);
    },
  },
});
