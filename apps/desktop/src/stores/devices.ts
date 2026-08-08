// 设备 store：本机信息 + 设备列表（UI_DESIGN_SPEC §9）。
// 数据来源：list_devices / get_self_device + device_* 事件。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type { DeviceInfo, PendingIntroduction } from "../lib/types";

interface State {
  self: DeviceInfo | null;
  devices: DeviceInfo[];
  /** 待确认的引荐（TRUST_DESIGN.md §5，里程碑 R2）。 */
  pendingIntroductions: PendingIntroduction[];
}

export const useDeviceStore = defineStore("devices", {
  state: (): State => ({ self: null, devices: [], pendingIntroductions: [] }),

  getters: {
    /** 可在界面显示的设备：在线的，或离线但已配对的（§5.1）。 */
    visible: (s): DeviceInfo[] =>
      s.devices.filter((d) => d.online || d.trusted),
  },

  actions: {
    async loadSelf() {
      this.self = await api.getSelfDevice();
    },
    async loadDevices() {
      this.devices = await api.listDevices();
    },
    upsert(device: DeviceInfo) {
      const i = this.devices.findIndex((d) => d.id === device.id);
      if (i >= 0) this.devices[i] = device;
      else this.devices.push(device);
    },
    markLost(id: string) {
      const i = this.devices.findIndex((d) => d.id === id);
      if (i < 0) return;
      // 离线后：已配对的保留并置灰，未配对的移除（§5.1）
      if (this.devices[i].trusted) {
        this.devices[i] = { ...this.devices[i], online: false, addr: null };
      } else {
        this.devices.splice(i, 1);
      }
    },
    /** 设备 id → 显示名（记录页用）。 */
    nameOf(id: string): string {
      return this.devices.find((d) => d.id === id)?.name ?? "未知设备";
    },

    // —— 信任传递 / 引荐（TRUST_DESIGN.md §5，里程碑 R2）——

    async loadPendingIntroductions() {
      this.pendingIntroductions = await api.listPendingIntroductions();
    },
    /** 确认「这也是我的设备」——升级为完全信任，随后刷新设备列表。 */
    async confirmIntroduction(deviceId: string) {
      await api.confirmIntroduction(deviceId);
      await Promise.all([this.loadPendingIntroductions(), this.loadDevices()]);
    },
    async dismissIntroduction(deviceId: string) {
      await api.dismissIntroduction(deviceId);
      await this.loadPendingIntroductions();
    },
  },
});
