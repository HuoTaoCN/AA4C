// 归档 store（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type { ArchiveEntry, ArchiveLogEntry, ArchiveRule } from "../lib/types";

interface State {
  rules: ArchiveRule[];
  entries: ArchiveEntry[];
  log: ArchiveLogEntry[];
  loading: boolean;
}

export const useArchiveStore = defineStore("archive", {
  state: (): State => ({ rules: [], entries: [], log: [], loading: false }),

  actions: {
    async loadRules() {
      this.rules = await api.listArchiveRules();
    },
    async loadEntries() {
      this.entries = await api.listArchiveEntries();
    },
    async loadLog() {
      this.log = await api.listArchiveLog();
    },
    async loadAll() {
      this.loading = true;
      try {
        await Promise.all([this.loadRules(), this.loadEntries(), this.loadLog()]);
      } finally {
        this.loading = false;
      }
    },

    async saveRule(rule: ArchiveRule): Promise<ArchiveRule> {
      const saved = await api.saveArchiveRule(rule);
      await this.loadRules();
      return saved;
    },
    async deleteRule(id: string) {
      await api.deleteArchiveRule(id);
      await this.loadRules();
    },

    /** 手动归档；`ruleId` 手选某条规则强制应用，`targetDir` 完全自定义目标目录，
     * 两者都不给时按启用规则自动匹配。返回实际归档成功的路径数。 */
    async archiveFiles(
      paths: string[],
      ruleId?: string,
      targetDir?: string,
    ): Promise<number> {
      const done = await api.archiveFiles(paths, ruleId, targetDir);
      await Promise.all([this.loadEntries(), this.loadLog()]);
      return done.length;
    },
    async undo(logId: number) {
      await api.undoArchive(logId);
      await Promise.all([this.loadEntries(), this.loadLog()]);
    },

    /** `ArchiveApplied` 事件到来时刷新（自动归档也会触发这条，不只是手动操作）。 */
    onApplied() {
      void this.loadEntries();
      void this.loadLog();
    },
  },
});
