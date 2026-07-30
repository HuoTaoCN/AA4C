// AI 模型库 store（V0.5 里程碑 AI2.4，ARCHIVE_DESIGN.md §3.5）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type { AiEngineStatePayload, AiStatus, LocalModel } from "../lib/types";

interface State {
  models: LocalModel[];
  status: AiStatus | null;
  loading: boolean;
}

export const useAiStore = defineStore("ai", {
  state: (): State => ({ models: [], status: null, loading: false }),

  actions: {
    async loadModels() {
      this.models = await api.listLocalModels();
    },
    async loadStatus() {
      this.status = await api.getAiStatus();
    },
    async loadAll() {
      this.loading = true;
      try {
        await Promise.all([this.loadModels(), this.loadStatus()]);
      } finally {
        this.loading = false;
      }
    },

    /** `AiEngineState` 事件到来时刷新状态——懒启动/空闲自停都经这条通知，
     * 不重新扫描模型目录（那部分没有变化）。 */
    onEngineState(_payload: AiEngineStatePayload) {
      void this.loadStatus();
    },
  },
});
