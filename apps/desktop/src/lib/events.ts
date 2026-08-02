// 事件桥接：把后端 aa4c:// 事件分发到对应 store，并触发通知 / toast。
// 在 App 根组件 onMounted 时调用一次（UI_DESIGN_SPEC §9）。

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { useAiStore } from "../stores/ai";
import { useArchiveStore } from "../stores/archive";
import { useDeviceStore } from "../stores/devices";
import { useDownloadStore } from "../stores/download";
import { usePairingStore } from "../stores/pairing";
import { useSettingsStore } from "../stores/settings";
import { useSyncStore } from "../stores/sync";
import { useToastStore } from "../stores/toast";
import { useTransferStore } from "../stores/transfer";
import type {
  AiEngineStatePayload,
  AiSuggestProgressPayload,
  ArchiveAppliedPayload,
  DeviceInfo,
  DeviceLostPayload,
  DownloadDonePayload,
  DownloadFailedPayload,
  DownloadProgressPayload,
  PairingPinPayload,
  PairingRequestPayload,
  PairingResultPayload,
  TransferConnectedPayload,
  TransferDonePayload,
  TransferFailedPayload,
  TransferProgressPayload,
  TransferRequestPayload,
} from "./types";

async function ensureNotificationPermission(): Promise<boolean> {
  let granted = await isPermissionGranted();
  if (!granted) {
    granted = (await requestPermission()) === "granted";
  }
  return granted;
}

function notify(title: string, body: string) {
  void ensureNotificationPermission().then((ok) => {
    if (ok) sendNotification({ title, body });
  });
}

/** 注册全部事件监听，返回取消函数。 */
export async function startEventBridge(): Promise<UnlistenFn> {
  const devices = useDeviceStore();
  const pairing = usePairingStore();
  const transfer = useTransferStore();
  const settings = useSettingsStore();
  const sync = useSyncStore();
  const toast = useToastStore();
  const download = useDownloadStore();
  const archive = useArchiveStore();
  const ai = useAiStore();

  const unlisten = await Promise.all([
    listen<DeviceInfo>("aa4c://device_found", (e) => devices.upsert(e.payload)),
    listen<DeviceInfo>("aa4c://device_updated", (e) => devices.upsert(e.payload)),
    listen<DeviceLostPayload>("aa4c://device_lost", (e) =>
      devices.markLost(e.payload.id),
    ),

    listen<PairingRequestPayload>("aa4c://pairing_request", (e) =>
      pairing.onRequest(e.payload.sessionId, e.payload.peer),
    ),
    listen<PairingPinPayload>("aa4c://pairing_pin", (e) =>
      pairing.onPin(e.payload.sessionId, e.payload.pin),
    ),
    listen<PairingResultPayload>("aa4c://pairing_result", (e) => {
      pairing.onResult(e.payload.sessionId, e.payload.success, e.payload.peer);
      void devices.loadDevices(); // 配对成功后刷新 trusted 标记
      // 成功后由「这是你自己的设备吗？」弹窗收尾，不再单独 toast；失败才提示
      if (!e.payload.success) {
        toast.push("error", "配对未完成，可以重新发起");
      }
    }),

    listen<TransferRequestPayload>("aa4c://transfer_request", (e) =>
      transfer.onRequest(e.payload.task),
    ),
    listen<TransferConnectedPayload>("aa4c://transfer_connected", (e) =>
      transfer.onConnected(e.payload),
    ),
    listen<TransferProgressPayload>("aa4c://transfer_progress", (e) =>
      transfer.onProgress(e.payload),
    ),
    listen<TransferDonePayload>("aa4c://transfer_done", (e) => {
      const before = transfer.active[e.payload.taskId];
      transfer.onDone(e.payload.taskId);
      if (before?.direction === "recv") {
        const peer = devices.nameOf(before.peer);
        const n = before.files.length;
        const msg = `已收到来自 ${peer} 的 ${n} 个文件`;
        toast.push("success", msg, settings.settings?.saveDir);
        notify("AA连接", msg);
      } else if (before?.direction === "send") {
        const peer = devices.nameOf(before.peer);
        toast.push("success", `已发送到 ${peer}`);
      }
    }),
    listen<TransferFailedPayload>("aa4c://transfer_failed", (e) => {
      // error 已是后端给出的人话消息（取消/拒绝等）
      transfer.onFailed(e.payload.taskId);
      toast.push("error", e.payload.error || "传输失败，请重试");
    }),

    listen<null>("aa4c://sync_index_updated", () => void sync.load()),

    listen<DownloadProgressPayload>("aa4c://download_progress", (e) =>
      download.onProgress(e.payload),
    ),
    listen<DownloadDonePayload>("aa4c://download_done", (e) => {
      download.onDone(e.payload);
      toast.push("success", "下载完成", e.payload.savePath);
      notify("AA连接", "下载完成");
    }),
    listen<DownloadFailedPayload>("aa4c://download_failed", (e) => {
      download.onFailed(e.payload);
      toast.push("error", e.payload.error || "下载失败，请重试");
    }),

    // 归档（里程碑 AI1）：自动（下载完成钩子）或手动归档都会发这条。刷新下载列表
    // 是因为归档会改写 download_tasks.save_path——不刷新的话"打开所在文件夹"
    // 会指向文件挪走前的旧位置（ARCHIVE_DESIGN.md §2.4 明确点出的这个坑）。
    listen<ArchiveAppliedPayload>("aa4c://archive_applied", () => {
      archive.onApplied();
      void download.load();
    }),

    // AI 引擎槽位状态（里程碑 AI2）：懒启动/空闲自停都经这条通知，模型库页
    // 用它刷新"加载中/就绪"这类状态展示。
    listen<AiEngineStatePayload>("aa4c://ai_engine_state", (e) =>
      ai.onEngineState(e.payload),
    ),

    // AI 建议批量进度（里程碑 AI3）：done>=total 时 store 自己拉取结果列表。
    listen<AiSuggestProgressPayload>("aa4c://ai_suggest_progress", (e) =>
      archive.onSuggestProgress(e.payload.done, e.payload.total),
    ),
  ]);

  return () => unlisten.forEach((fn) => fn());
}
