<script setup lang="ts">
// 桌面外壳：顶栏 + 侧导航 + 内容区 + 全局任务条（UI_DESIGN_SPEC §2）。
import { storeToRefs } from "pinia";
import { useDeviceStore } from "../stores/devices";
import { usePluginStore } from "../stores/plugins";
import { useTransferStore } from "../stores/transfer";
import TaskBar from "./TaskBar.vue";
import { Icon, StatusDot } from "./ui";
import { PRIMARY, SETTINGS } from "../lib/nav";

const devices = useDeviceStore();
const plugins = usePluginStore();
const transfer = useTransferStore();
const { self } = storeToRefs(devices);
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand">AA连接</div>
      <div v-if="self" class="self">
        <span>{{ self.name }}</span>
        <StatusDot tone="ok" />
      </div>
    </header>

    <div class="body">
      <nav class="sidenav">
        <div class="group">
          <router-link
            v-for="it in PRIMARY"
            :key="it.path"
            :to="it.path"
            class="navitem"
            :class="{ active: $route.path === it.path }"
          >
            <Icon :name="it.icon" />
            <span class="t">{{ it.name }}</span>
          </router-link>
        </div>

        <!-- 「更多」：装了插件才出现（UI_DESIGN_SPEC §2）。
             空的插件注册表应当得到一个只有四项导航的干净界面。 -->
        <template v-if="plugins.navItems.length">
          <div class="sep">更多</div>
          <div class="group">
            <router-link
              v-for="it in plugins.navItems"
              :key="it.path"
              :to="it.path"
              class="navitem"
              :class="{ active: $route.path === it.path }"
            >
              <Icon :name="it.icon" />
              <span class="t">{{ it.name }}</span>
            </router-link>
          </div>
        </template>

        <div class="spacer" />

        <router-link
          :to="SETTINGS.path"
          class="navitem"
          :class="{ active: $route.path === SETTINGS.path }"
        >
          <Icon :name="SETTINGS.icon" />
          <span class="t">{{ SETTINGS.name }}</span>
        </router-link>
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
  gap: var(--sp-4);
  height: 52px;
  padding: 0 var(--sp-5);
  background: var(--aa-surface);
  border-bottom: 1px solid var(--aa-border);
  flex-shrink: 0;
}
.brand {
  font-weight: var(--fw-bold);
  font-size: var(--fs-lg);
  letter-spacing: 0.03em;
}
.self {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-sm);
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
  padding: var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  border-right: 1px solid var(--aa-border);
  background: var(--aa-surface);
}
.group {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.sep {
  margin: var(--sp-4) var(--sp-3) var(--sp-1);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.spacer {
  flex: 1;
}
.navitem {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--aa-radius-sm);
  font-weight: var(--fw-bold);
  font-size: var(--fs-base);
  color: var(--aa-text-dim);
  transition: background var(--motion-fast);
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
.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: var(--sp-5) var(--sp-6);
}
</style>
