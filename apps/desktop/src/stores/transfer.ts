// 传输 store：进行中任务 + 历史记录 + 待接收请求（UI_DESIGN_SPEC §9）。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  ConnectionVia,
  DeviceInfo,
  TransferConnectedPayload,
  TransferProgressPayload,
  TransferTask,
} from "../lib/types";

/** 进行中任务：在 TransferTask 上叠加实时速度、当前文件、连接方式（里程碑 C4）。 */
export interface ActiveTask extends TransferTask {
  speedBps: number;
  currentFile: string;
  /** 这次连接实际走的档位；事件到达前为 undefined（只有发起方会收到，见后端文档）。 */
  via?: ConnectionVia;
}

interface State {
  active: Record<string, ActiveTask>;
  incoming: Record<string, TransferTask>;
  history: TransferTask[];
}

function skeleton(
  id: string,
  direction: TransferTask["direction"],
  peer: string,
): ActiveTask {
  return {
    id,
    direction,
    peer,
    files: [],
    status: "waiting_accept",
    totalBytes: 0,
    transferredBytes: 0,
    createdAt: Date.now(),
    error: null,
    speedBps: 0,
    currentFile: "",
  };
}

export const useTransferStore = defineStore("transfer", {
  state: (): State => ({ active: {}, incoming: {}, history: [] }),

  getters: {
    activeList: (s): ActiveTask[] => Object.values(s.active),
    incomingList: (s): TransferTask[] => Object.values(s.incoming),
    hasActive: (s): boolean => Object.keys(s.active).length > 0,
  },

  actions: {
    async loadHistory() {
      this.history = await api.listTransfers(50, 0);
    },

    /** 发起发送，返回 taskId，并登记一个等待确认的占位任务。 */
    async send(device: DeviceInfo, paths: string[]): Promise<string> {
      const taskId = await api.sendFiles(device.id, paths);
      this.active[taskId] = skeleton(taskId, "send", device.id);
      return taskId;
    },

    /** 出站连接已建立（只有发起方——发送/拉取——会收到，见后端 `CoreEvent` 文档，里程碑 C4）。
     *  拉取（`sync.fetch`）不会预先登记占位任务，这里和 `onProgress` 一样懒创建，
     *  否则这条早于第一次 `TransferProgress` 到达的事件会因为找不到任务而白白丢掉。 */
    onConnected(p: TransferConnectedPayload) {
      const task = this.active[p.taskId] ?? skeleton(p.taskId, "recv", "");
      task.via = p.via;
      this.active[p.taskId] = task;
    },

    /** 收到对方的发送请求（待用户确认）。 */
    onRequest(task: TransferTask) {
      this.incoming[task.id] = task;
    },

    /** 接收端确认 / 拒绝。接受后任务转入进行中。 */
    async accept(taskId: string, accept: boolean, saveDir?: string) {
      await api.acceptTransfer(taskId, accept, saveDir);
      const task = this.incoming[taskId];
      delete this.incoming[taskId];
      if (accept && task) {
        this.active[taskId] = { ...task, status: "transferring", speedBps: 0, currentFile: "" };
      }
    },

    onProgress(p: TransferProgressPayload) {
      const base =
        this.active[p.taskId] ??
        (this.incoming[p.taskId]
          ? { ...this.incoming[p.taskId], speedBps: 0, currentFile: "" }
          : skeleton(p.taskId, "recv", ""));
      this.active[p.taskId] = {
        ...base,
        status: "transferring",
        transferredBytes: p.transferredBytes,
        totalBytes: p.totalBytes || base.totalBytes,
        speedBps: p.speedBps,
        currentFile: p.currentFile,
      };
    },

    onDone(taskId: string) {
      delete this.active[taskId];
      delete this.incoming[taskId];
      void this.loadHistory();
    },

    onFailed(taskId: string) {
      delete this.active[taskId];
      delete this.incoming[taskId];
      void this.loadHistory();
    },

    /** 暂停后任务留在列表里（状态转 paused），不像完成/失败那样移出——
     *  用户还要在那一行上点「继续」。 */
    onPaused(taskId: string) {
      const task = this.active[taskId];
      if (task) {
        this.active[taskId] = { ...task, status: "paused", speedBps: 0 };
      }
      void this.loadHistory();
    },

    async cancel(taskId: string) {
      await api.cancelTransfer(taskId);
    },
    async pause(taskId: string) {
      await api.pauseTransfer(taskId);
    },
    async resume(taskId: string) {
      await api.resumeTransfer(taskId);
    },
  },
});
