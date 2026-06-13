<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { storeToRefs } from "pinia";

import TaskBar from "./components/TaskBar.vue";
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
const { self } = storeToRefs(devices);

const nav = [
  { to: "/", label: "首页", icon: "🏠" },
  { to: "/send", label: "AA", icon: "📤" },
  { to: "/records", label: "记录", icon: "📋" },
  { to: "/settings", label: "设置", icon: "⚙️" },
];

let unlisten: UnlistenFn | null = null;
let poll: number | null = null;

onMounted(async () => {
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
  // 安全网：mDNS 事件之外，定期刷新设备在线状态
  poll = window.setInterval(() => {
    devices.loadDevices().catch(() => {});
  }, 5000);
});

onUnmounted(() => {
  unlisten?.();
  if (poll !== null) window.clearInterval(poll);
});
</script>

<template>
  <div class="app">
    <!-- 顶栏 -->
    <header class="topbar">
      <div class="brand"><span class="dot">◉</span> AA4C</div>
      <div class="self" v-if="self">
        <span>{{ self.name }}</span>
        <span class="online-dot" title="在线"></span>
      </div>
      <router-link to="/settings" class="gear" title="设置">⚙️</router-link>
    </header>

    <div class="body">
      <!-- 侧边导航（桌面） -->
      <nav class="sidenav">
        <router-link
          v-for="item in nav.slice(0, 3)"
          :key="item.to"
          :to="item.to"
          class="navitem"
          active-class="active"
        >
          <span class="i">{{ item.icon }}</span>{{ item.label }}
        </router-link>
      </nav>

      <main class="content">
        <router-view />
      </main>
    </div>

    <!-- 全局任务条 -->
    <TaskBar v-if="transfer.hasActive" />

    <!-- 底部导航（移动端 < 700px） -->
    <nav class="bottomnav">
      <router-link
        v-for="item in nav"
        :key="item.to"
        :to="item.to"
        class="tab"
        active-class="active"
      >
        <span class="i">{{ item.icon }}</span>
        <span class="t">{{ item.label }}</span>
      </router-link>
    </nav>

    <!-- 弹窗与提示 -->
    <PairingDialog />
    <ReceiveDialog />
    <ToastHost />
  </div>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}

.topbar {
  display: flex;
  align-items: center;
  gap: 16px;
  height: 52px;
  padding: 0 18px;
  background: var(--aa-surface);
  border-bottom: 1px solid var(--aa-border);
  flex-shrink: 0;
}
.brand {
  font-weight: 800;
  font-size: 1.05rem;
  letter-spacing: 0.03em;
}
.brand .dot {
  color: var(--aa-primary);
}
.self {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 0.9rem;
  color: var(--aa-text-dim);
}
.gear {
  font-size: 1.1rem;
}

.body {
  display: flex;
  flex: 1;
  min-height: 0;
}

.sidenav {
  width: 168px;
  flex-shrink: 0;
  padding: 14px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  border-right: 1px solid var(--aa-border);
  background: var(--aa-surface);
}
.navitem {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-radius: var(--aa-radius-sm);
  font-weight: 600;
  font-size: 0.95rem;
  color: var(--aa-text-dim);
}
.navitem:hover {
  background: var(--aa-surface-2);
}
.navitem.active {
  background: var(--aa-primary-dim);
  color: var(--aa-primary);
}

.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 22px 26px;
}

.bottomnav {
  display: none;
}

/* 移动布局（UI_DESIGN_SPEC §10） */
@media (max-width: 700px) {
  .sidenav {
    display: none;
  }
  .bottomnav {
    display: flex;
    flex-shrink: 0;
    border-top: 1px solid var(--aa-border);
    background: var(--aa-surface);
    padding-bottom: env(safe-area-inset-bottom);
  }
  .tab {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 0;
    min-height: 52px;
    font-size: 0.72rem;
    color: var(--aa-text-dim);
  }
  .tab.active {
    color: var(--aa-primary);
  }
  .tab .i {
    font-size: 1.2rem;
  }
  .content {
    padding: 16px;
  }
}
</style>
