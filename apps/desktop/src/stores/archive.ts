// 归档 store（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  ArchiveEntry,
  ArchiveLogEntry,
  ArchiveRule,
  Suggestion,
} from "../lib/types";

interface State {
  rules: ArchiveRule[];
  entries: ArchiveEntry[];
  log: ArchiveLogEntry[];
  loading: boolean;
  suggestions: Suggestion[];
  suggestRunning: boolean;
  suggestDone: number;
  suggestTotal: number;
}

export const useArchiveStore = defineStore("archive", {
  state: (): State => ({
    rules: [],
    entries: [],
    log: [],
    loading: false,
    suggestions: [],
    suggestRunning: false,
    suggestDone: 0,
    suggestTotal: 0,
  }),

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

    // —— AI 建议（里程碑 AI3，ARCHIVE_DESIGN.md §5）——

    /** 起一批建议：进度经 `onSuggestProgress` 事件更新，结果调用方自己后续
     * `loadSuggestions()` 拉取（批量跑完时机由事件的 done===total 判断）。 */
    async startSuggest(paths: string[]) {
      this.suggestRunning = true;
      this.suggestDone = 0;
      this.suggestTotal = paths.length;
      await api.startSuggest(paths);
    },
    async loadSuggestions() {
      this.suggestions = await api.listSuggestions();
    },
    /** 采纳（`adopt=true`，可选 `targetDir` 顺带移动）或忽略一条建议；两种情况
     * 都从本地列表摘掉这一条（忽略=丢弃，ARCHIVE_DESIGN.md §5），采纳时顺带
     * 刷新归档记录/日志。 */
    async resolveSuggestion(id: string, adopt: boolean, targetDir?: string) {
      await api.resolveSuggestion(id, adopt, targetDir);
      this.suggestions = this.suggestions.filter((s) => s.id !== id);
      if (adopt) {
        await Promise.all([this.loadEntries(), this.loadLog()]);
      }
    },
    /** `AiSuggestProgress` 事件到来时更新进度；跑完（done>=total）后拉取结果。 */
    onSuggestProgress(done: number, total: number) {
      this.suggestDone = done;
      this.suggestTotal = total;
      if (done >= total) {
        this.suggestRunning = false;
        void this.loadSuggestions();
      }
    },
  },
});
