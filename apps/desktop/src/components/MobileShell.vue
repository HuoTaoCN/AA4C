<script setup lang="ts">
// 移动外壳（UI_DESIGN_SPEC §11）。
//
// 底部 tab 与桌面主导航**一一对应**。F3 之前是 5 个 tab，其中「我的」是个把记录、
// 分享、归档、设置四样塞在一起的杂物抽屉——那是桌面侧导航过长的连锁后果，
// 主导航收到 5 项之后它就没有存在理由了。插件页面走顶部「更多」，不占底部 tab。
import { computed } from "vue";
import { useTransferStore } from "../stores/transfer";
import { usePluginStore } from "../stores/plugins";
import TaskBar from "./TaskBar.vue";
import { Icon } from "./ui";
import { PRIMARY, SETTINGS } from "../lib/nav";

const transfer = useTransferStore();
const plugins = usePluginStore();

// 底部只放 4 个：设备 / 传输 / 同步 / 设置。分享在移动端进「更多」——
// 触屏上 5 个 tab 已经偏挤，而分享的使用频率明显低于前三项。
const tabs = computed(() => [...PRIMARY.slice(0, 3), SETTINGS]);
const more = computed(() => [PRIMARY[3], ...plugins.navItems]);
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand">AA连接</div>
      <nav class="more">
        <router-link
          v-for="it in more"
          :key="it.path"
          :to="it.path"
          class="morelink"
          :class="{ active: $route.path === it.path }"
          :title="it.name"
        >
          <Icon :name="it.icon" :size="16" />
        </router-link>
      </nav>
    </header>

    <main class="content"><router-view /></main>

    <TaskBar v-if="transfer.hasActive" />

    <nav class="tabbar">
      <router-link
        v-for="it in tabs"
        :key="it.path"
        :to="it.path"
        class="tab"
        :class="{ active: $route.path === it.path }"
      >
        <Icon :name="it.icon" :size="20" />
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
  padding: 0 var(--sp-4);
  background: var(--aa-surface);
  border-bottom: 1px solid var(--aa-border);
  flex-shrink: 0;
}
.brand {
  font-weight: var(--fw-bold);
  font-size: var(--fs-lg);
}
.more {
  margin-left: auto;
  display: flex;
  gap: var(--sp-1);
}
.morelink {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--aa-radius-sm);
  color: var(--aa-text-dim);
}
.morelink.active {
  background: var(--aa-primary-dim);
  color: var(--aa-primary);
}
.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: var(--sp-4);
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
  gap: var(--sp-1);
  padding: var(--sp-2) 0;
  /* 触控目标最小 44×44（§11） */
  min-height: 52px;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.tab.active {
  color: var(--aa-primary);
}
</style>
