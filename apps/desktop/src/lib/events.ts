// 事件桥接：把后端 aa4c:// 事件分发到对应 store，并触发通知 / toast。
// 在 App 根组件 onMounted 时调用一次（UI_DESIGN_SPEC §9）。

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { useDeviceStore } from "../stores/devices";
import { usePairingStore } from "../stores/pairing";
import { useSettingsStore } from "../stores/settings";
import { useToastStore } from "../stores/toast";
import { useTransferStore } from "../stores/transfer";
import type {
  DeviceInfo,
  DeviceLostPayload,
  PairingPinPayload,
  PairingRequestPayload,
  PairingResultPayload,
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
  const toast = useToastStore();

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
      pairing.onResult(e.payload.sessionId, e.payload.success);
      void devices.loadDevices(); // 配对成功后刷新 trusted 标记
      const r = pairing.lastResult;
      if (r) {
        toast.push(
          r.success ? "success" : "error",
          r.success ? `已和 ${r.peerName} 配对成功 🎉` : "配对未完成，可以重新发起",
        );
      }
    }),

    listen<TransferRequestPayload>("aa4c://transfer_request", (e) =>
      transfer.onRequest(e.payload.task),
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
  ]);

  return () => unlisten.forEach((fn) => fn());
}
