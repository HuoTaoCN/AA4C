<script setup lang="ts">
import { useTransferStore } from "../stores/transfer";
import TaskBar from "./TaskBar.vue";
import { MOBILE_TABS } from "../lib/nav";

const transfer = useTransferStore();
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand"><span class="dot">◉</span> AA连接</div>
    </header>

    <main class="content"><router-view /></main>

    <TaskBar v-if="transfer.hasActive" />

    <nav class="tabbar">
      <router-link
        v-for="it in MOBILE_TABS"
        :key="it.path"
        :to="it.path"
        class="tab"
        :class="{ active: $route.path === it.path }"
      >
        <span class="i">{{ it.icon }}</span>
        <span class="t">{{ it.name }}</span>
      </router-link>
    </nav>
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
  height: 48px;
  padding: 0 16px;
  background: var(--aa-surface);
  border-bottom: 1px solid var(--aa-border);
  flex-shrink: 0;
}
.brand {
  font-weight: 800;
  font-size: 1.02rem;
}
.brand .dot {
  color: var(--aa-primary);
}
.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 16px;
}
.tabbar {
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
</style>
