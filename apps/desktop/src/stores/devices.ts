// 设备 store：本机信息 + 设备列表（UI_DESIGN_SPEC §9）。
// 数据来源：list_devices / get_self_device + device_* 事件。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type {
  DeviceInfo,
  DeviceReachability,
  PendingIntroduction,
  TrustLevel,
} from "../lib/types";

interface State {
  self: DeviceInfo | null;
  devices: DeviceInfo[];
  /** 待确认的引荐（TRUST_DESIGN.md §5，里程碑 R2）。 */
  pendingIntroductions: PendingIntroduction[];
  /** 每台已配对设备当下的可达性（V0.8 F2）：连不连得上、走的哪一档。 */
  reachability: DeviceReachability[];
}

export const useDeviceStore = defineStore("devices", {
  state: (): State => ({
    self: null,
    devices: [],
    pendingIntroductions: [],
    reachability: [],
  }),

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

    // —— 可达性（V0.8「Focus」F2）——

    async loadReachability() {
      this.reachability = await api.listReachability();
    },
    /** 设备 id → 可达性。查不到时返回 `undefined`，调用方按「还不知道」处理，
     * **不要**替它编一个「离线」。 */
    reachOf(id: string): DeviceReachability | undefined {
      return this.reachability.find((r) => r.deviceId === id);
    },

    /** 升 / 降完全信任。升级后后端会立刻拉一次对端索引（顺带就是一次可达性探测），
     * 所以两份数据都要重拉。 */
    async setTrustLevel(deviceId: string, level: TrustLevel) {
      await api.setTrustLevel(deviceId, level);
      await Promise.all([this.loadDevices(), this.loadReachability()]);
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
