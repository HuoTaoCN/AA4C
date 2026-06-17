<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useDeviceStore } from "../stores/devices";
import { useToastStore } from "../stores/toast";
import { platformIcon } from "../lib/format";
import { CAPABILITIES, UTILITY } from "../lib/nav";

const devices = useDeviceStore();
const toast = useToastStore();
const { self } = storeToRefs(devices);

// 「我的」聚合：底部标签放不下的能力（分享 / 归档）+ 次级入口（记录 / 设置）
const share = CAPABILITIES.find((c) => c.path === "/share")!;
const archive = CAPABILITIES.find((c) => c.path === "/archive")!;
const entries = [share, archive, ...UTILITY];

function about() {
  toast.push("info", "AA连接 —— 连接你的所有设备");
}
</script>

<template>
  <div class="me">
    <h2>我的</h2>

    <section v-if="self" class="self card">
      <div class="icon">{{ platformIcon(self.platform) }}</div>
      <div>
        <div class="name">{{ self.name }}</div>
        <div class="sub muted">本机 · 在线</div>
      </div>
    </section>

    <ul class="list card">
      <router-link v-for="e in entries" :key="e.path" :to="e.path" class="row" custom v-slot="{ navigate }">
        <li @click="navigate">
          <span class="ic">{{ e.icon }}</span>
          <span class="nm">{{ e.name }}</span>
          <span v-if="!e.built" class="tag">建设中</span>
          <span class="chev">›</span>
        </li>
      </router-link>
      <li @click="about">
        <span class="ic">ℹ️</span>
        <span class="nm">关于</span>
        <span class="chev">›</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.me {
  max-width: 640px;
  margin: 0 auto;
}
h2 {
  font-size: 1rem;
  margin: 0 0 16px;
}
.self {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 18px;
  margin-bottom: 16px;
}
.self .icon {
  font-size: 2rem;
}
.self .name {
  font-weight: 700;
  font-size: 1.05rem;
}
.self .sub {
  font-size: 0.82rem;
}
.list {
  list-style: none;
  margin: 0;
  padding: 4px 0;
}
.list li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 13px 16px;
  cursor: pointer;
  font-size: 0.95rem;
}
.list li + li {
  border-top: 1px solid var(--aa-border);
}
.list li:hover {
  background: var(--aa-surface-2);
}
.ic {
  font-size: 1.2rem;
}
.nm {
  font-weight: 500;
}
.tag {
  font-size: 0.7rem;
  color: var(--aa-text-dim);
  background: var(--aa-surface-2);
  padding: 1px 7px;
  border-radius: 999px;
}
.chev {
  margin-left: auto;
  color: var(--aa-text-dim);
  font-size: 1.2rem;
}
</style>
