<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useDeviceStore } from "../stores/devices";
import { useTransferStore } from "../stores/transfer";
import TaskBar from "./TaskBar.vue";
import { HOME, CAPABILITIES, UTILITY } from "../lib/nav";

const devices = useDeviceStore();
const transfer = useTransferStore();
const { self } = storeToRefs(devices);

// 主导航：首页 + 五大能力
const primary = [HOME, ...CAPABILITIES];
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand"><span class="dot">◉</span> AA连接</div>
      <div class="self" v-if="self">
        <span>{{ self.name }}</span>
        <span class="online-dot" title="在线"></span>
      </div>
    </header>

    <div class="body">
      <nav class="sidenav">
        <div class="group">
          <router-link
            v-for="it in primary"
            :key="it.path"
            :to="it.path"
            class="navitem"
            :class="{ active: $route.path === it.path }"
          >
            <span class="i">{{ it.icon }}</span>
            <span class="t">{{ it.name }}</span>
            <span v-if="!it.built" class="tag">建设中</span>
          </router-link>
        </div>

        <div class="spacer"></div>

        <div class="group">
          <router-link
            v-for="it in UTILITY"
            :key="it.path"
            :to="it.path"
            class="navitem"
            :class="{ active: $route.path === it.path }"
          >
            <span class="i">{{ it.icon }}</span>
            <span class="t">{{ it.name }}</span>
          </router-link>
        </div>
      </nav>

      <main class="content"><router-view /></main>
    </div>

    <TaskBar v-if="transfer.hasActive" />
  </div>
</template>

<style scoped>
.shell {
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
.body {
  display: flex;
  flex: 1;
  min-height: 0;
}
.sidenav {
  width: 176px;
  flex-shrink: 0;
  padding: 14px 12px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  border-right: 1px solid var(--aa-border);
  background: var(--aa-surface);
}
.group {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.spacer {
  flex: 1;
}
.navitem {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border-radius: var(--aa-radius-sm);
  font-weight: 600;
  font-size: 0.92rem;
  color: var(--aa-text-dim);
}
.navitem:hover {
  background: var(--aa-surface-2);
}
.navitem.active {
  background: var(--aa-primary-dim);
  color: var(--aa-primary);
}
.navitem .t {
  flex: 1;
}
.tag {
  font-size: 0.62rem;
  font-weight: 600;
  color: var(--aa-text-dim);
  background: var(--aa-bg);
  padding: 1px 6px;
  border-radius: 999px;
}
.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 22px 26px;
}
</style>
