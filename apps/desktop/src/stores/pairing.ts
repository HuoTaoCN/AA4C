// 配对 store（UI_DESIGN_SPEC §4.2 / §9）。
//
// 一个配对会话经历：请求（仅接收方）→ 显示确认码 → 结果。
// 发起方没有「请求」环节，直接等确认码。

import { defineStore } from "pinia";
import { api } from "../lib/api";
import type { DeviceInfo } from "../lib/types";

export interface PairingSession {
  sessionId: string;
  peerName: string;
  /** 接收方收到请求、尚未接受时为 true（需要弹「接受/拒绝」）。 */
  needsAccept: boolean;
  /** 双方确认码（出现后弹「确认码一致？」）。 */
  pin?: string;
}

interface State {
  sessions: Record<string, PairingSession>;
  lastResult: { success: boolean; peerName: string } | null;
  /** 配对成功后追问「这是你自己的设备吗？」（信任分级，预览）。 */
  trustPrompt: { peerId: string; peerName: string } | null;
}

export const usePairingStore = defineStore("pairing", {
  state: (): State => ({ sessions: {}, lastResult: null, trustPrompt: null }),

  getters: {
    /** 需要展示确认码的会话（优先级高于接受请求）。 */
    pinSession: (s): PairingSession | null =>
      Object.values(s.sessions).find((x) => x.pin) ?? null,
    /** 需要接受配对请求的会话。 */
    requestSession: (s): PairingSession | null =>
      Object.values(s.sessions).find((x) => x.needsAccept && !x.pin) ?? null,
  },

  actions: {
    /** 发起方：向设备发起配对。 */
    async start(device: DeviceInfo) {
      const sessionId = await api.startPairing(device.id);
      this.sessions[sessionId] = {
        sessionId,
        peerName: device.name,
        needsAccept: false,
      };
    },
    /** 接收方：收到配对请求。 */
    onRequest(sessionId: string, peer: DeviceInfo) {
      this.sessions[sessionId] = {
        sessionId,
        peerName: peer.name,
        needsAccept: true,
      };
    },
    /** 双方：收到确认码。 */
    onPin(sessionId: string, pin: string) {
      const existing = this.sessions[sessionId];
      this.sessions[sessionId] = existing
        ? { ...existing, pin }
        : { sessionId, peerName: "对方设备", needsAccept: false, pin };
    },
    /** 用户对某会话做出确认 / 拒绝。 */
    async confirm(sessionId: string, accept: boolean) {
      await api.confirmPairing(sessionId, accept);
      if (!accept) delete this.sessions[sessionId];
      else {
        // 接受请求后清除 needsAccept，等待确认码
        const s = this.sessions[sessionId];
        if (s) this.sessions[sessionId] = { ...s, needsAccept: false };
      }
    },
    /** 双方：配对结束。成功则追问信任分级。 */
    onResult(sessionId: string, success: boolean, peerId: string) {
      const peerName = this.sessions[sessionId]?.peerName ?? "对方设备";
      delete this.sessions[sessionId];
      this.lastResult = { success, peerName };
      if (success) this.trustPrompt = { peerId, peerName };
    },
    /** 用户回答「这是你自己的设备吗？」（preview：暂只本地，trust_level 后端 V0.2）。 */
    resolveTrust(_tier: "full" | "friend") {
      this.trustPrompt = null;
    },
    clearResult() {
      this.lastResult = null;
    },
  },
});
