// 本地知识库 store（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  KbAnswerDonePayload,
  KbAnswerSource,
  KbIngestProgressPayload,
  KbSourceSummary,
} from "../lib/types";

interface State {
  sources: KbSourceSummary[];
  loading: boolean;
  /** 当前在跑的摄入（后端一次只允许一个），`null` = 没有摄入在跑。 */
  activeIngest: { sourceId: string; done: number; total: number } | null;
  /** 当前这次问答的 `request_id`——事件到来时用它判断是不是自己等的那条
   * （旧请求的迟到事件会被丢弃，不会污染新问题的回答）。 */
  currentRequestId: string | null;
  asking: boolean;
  answer: string;
  answerSources: KbAnswerSource[];
  answerError: string | null;
}

export const useKbStore = defineStore("kb", {
  state: (): State => ({
    sources: [],
    loading: false,
    activeIngest: null,
    currentRequestId: null,
    asking: false,
    answer: "",
    answerSources: [],
    answerError: null,
  }),

  actions: {
    async loadSources() {
      this.loading = true;
      try {
        this.sources = await api.kbListSources();
      } finally {
        this.loading = false;
      }
    },

    async addSource(path: string) {
      await api.kbAddSource(path);
      await this.loadSources();
    },

    async removeSource(id: string) {
      await api.kbRemoveSource(id);
      await this.loadSources();
    },

    async reindex(sourceId: string) {
      this.activeIngest = { sourceId, done: 0, total: 0 };
      await api.kbReindex(sourceId);
    },

    /** `KbIngestProgress` 事件到来时更新进度；跑完（done>=total）后清掉进度态、
     * 刷新来源摘要（文档计数/状态变了）。 */
    onIngestProgress(payload: KbIngestProgressPayload) {
      this.activeIngest = {
        sourceId: payload.sourceId,
        done: payload.done,
        total: payload.total,
      };
      if (payload.done >= payload.total) {
        this.activeIngest = null;
        void this.loadSources();
      }
    },

    async ask(question: string) {
      this.answer = "";
      this.answerSources = [];
      this.answerError = null;
      this.asking = true;
      this.currentRequestId = await api.kbAsk(question);
    },

    /** `KbAnswerDelta` 到来时追加内容；忽略不是当前这次提问的迟到增量。 */
    onAnswerDelta(payload: { requestId: string; delta: string }) {
      if (payload.requestId !== this.currentRequestId) return;
      this.answer += payload.delta;
    },

    onAnswerDone(payload: KbAnswerDonePayload) {
      if (payload.requestId !== this.currentRequestId) return;
      this.asking = false;
      this.answerSources = payload.sources;
      this.answerError = payload.error ?? null;
    },
  },
});
