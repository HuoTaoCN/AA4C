// 同步 store（SYNC_DESIGN.md §10 里程碑 2–3）：共享范围管理 + 跨设备统一文件视图。

import { defineStore } from "pinia";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import type { SyncScope, UnifiedFile } from "../lib/types";

interface State {
  scopes: SyncScope[];
  files: UnifiedFile[];
  loading: boolean;
}

export const useSyncStore = defineStore("sync", {
  state: (): State => ({ scopes: [], files: [], loading: false }),

  actions: {
    async load() {
      this.loading = true;
      try {
        const [scopes, files] = await Promise.all([
          api.listSyncScopes(),
          api.listUnifiedFiles(),
        ]);
        this.scopes = scopes;
        this.files = files;
      } finally {
        this.loading = false;
      }
    },
    /** 打开系统目录选择器，添加为同步文件夹（取消返回 null）。 */
    async addFolder(): Promise<SyncScope | null> {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked !== "string") return null;
      const scope = await api.addSyncScope(picked);
      await this.load();
      return scope;
    },
    async removeScope(id: string) {
      await api.removeSyncScope(id);
      await this.load();
    },
    /** 重新扫描本机共享范围（本地文件增删改）。 */
    async rescan() {
      await api.rescanSync();
      await this.load();
    },
    /** 与在线的「自己的设备」刷新一次跨设备索引（黄/红状态）。 */
    async refresh() {
      await api.refreshRemoteIndex();
      await this.load();
    },
  },
});
