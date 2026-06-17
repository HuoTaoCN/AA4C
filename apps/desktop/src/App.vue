<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";

import DesktopShell from "./components/DesktopShell.vue";
import MobileShell from "./components/MobileShell.vue";
import PairingDialog from "./components/PairingDialog.vue";
import ReceiveDialog from "./components/ReceiveDialog.vue";
import ToastHost from "./components/ToastHost.vue";

import { useDeviceStore } from "./stores/devices";
import { useSettingsStore } from "./stores/settings";
import { useTransferStore } from "./stores/transfer";
import { useToastStore } from "./stores/toast";
import { startEventBridge } from "./lib/events";
import { asCommandError } from "./lib/api";

const devices = useDeviceStore();
const settings = useSettingsStore();
const transfer = useTransferStore();
const toast = useToastStore();

// PC / 移动按视口切换两套外壳（UI_DESIGN_SPEC §10：断点 700px）
const MOBILE_BREAKPOINT = 700;
const isMobile = ref(window.innerWidth < MOBILE_BREAKPOINT);
function onResize() {
  isMobile.value = window.innerWidth < MOBILE_BREAKPOINT;
}

let unlisten: UnlistenFn | null = null;
let poll: number | null = null;

onMounted(async () => {
  window.addEventListener("resize", onResize);
  try {
    await Promise.all([
      devices.loadSelf(),
      devices.loadDevices(),
      settings.load(),
      transfer.loadHistory(),
    ]);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
  unlisten = await startEventBridge();
  poll = window.setInterval(() => {
    devices.loadDevices().catch(() => {});
  }, 5000);
});

onUnmounted(() => {
  window.removeEventListener("resize", onResize);
  unlisten?.();
  if (poll !== null) window.clearInterval(poll);
});
</script>

<template>
  <MobileShell v-if="isMobile" />
  <DesktopShell v-else />

  <PairingDialog />
  <ReceiveDialog />
  <ToastHost />
</template>
