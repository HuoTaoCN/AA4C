<script setup lang="ts">
import { computed } from "vue";
import { storeToRefs } from "pinia";
import { useDeviceStore } from "../stores/devices";
import { useTransferStore } from "../stores/transfer";
import DeviceCard from "../components/DeviceCard.vue";
import { CAPABILITIES } from "../lib/nav";
import { baseName, platformIcon, statusText, timeText } from "../lib/format";

const devices = useDeviceStore();
const transfer = useTransferStore();
const { self } = storeToRefs(devices);

const nearby = computed(() => devices.visible.filter((d) => d.id !== self.value?.id));
const recent = computed(() => transfer.history.slice(0, 5));

function summary(files: { relPath: string }[]): string {
  if (files.length === 0) return "文件";
  const first = baseName(files[0].relPath);
  return files.length === 1 ? first : `${first} 等 ${files.length} 个文件`;
}
</script>

<template>
  <div class="home">
    <!-- 本机卡片 -->
    <section v-if="self" class="self card">
      <div class="icon">{{ platformIcon(self.platform) }}</div>
      <div>
        <div class="name">{{ self.name }}</div>
        <div class="status"><span class="online-dot"></span> 在线 · 本机</div>
      </div>
    </section>

    <!-- 能力入口 -->
    <section>
      <h2>能力</h2>
      <div class="caps">
        <router-link
          v-for="c in CAPABILITIES"
          :key="c.path"
          :to="c.path"
          class="cap card"
        >
          <div class="cap-top">
            <span class="cap-ic">{{ c.icon }}</span>
            <span class="cap-pill" :class="c.built ? 'ok' : 'soon'">
              {{ c.built ? "可用" : "建设中" }}
            </span>
          </div>
          <div class="cap-nm">{{ c.name }}</div>
          <div class="cap-ds muted">{{ c.desc }}</div>
        </router-link>
      </div>
    </section>

    <!-- 附近设备 -->
    <section>
      <h2>附近设备</h2>
      <div v-if="nearby.length" class="grid">
        <DeviceCard v-for="d in nearby" :key="d.id" :device="d" />
      </div>
      <div v-else class="empty card">
        <p>附近还没有发现设备。</p>
        <p class="muted">在另一台设备上打开 AA连接，并连接同一个 WiFi。</p>
      </div>
    </section>

    <!-- 最近传输 -->
    <section v-if="recent.length">
      <div class="head">
        <h2>最近传输</h2>
        <router-link to="/records" class="more">查看全部</router-link>
      </div>
      <ul class="recent card">
        <li v-for="t in recent" :key="t.id">
          <span class="dir">{{ t.direction === "send" ? "⬆" : "⬇" }}</span>
          <span class="rname">{{ devices.nameOf(t.peer) }}</span>
          <span class="rfiles muted">{{ summary(t.files) }}</span>
          <span class="rstatus muted">{{ statusText(t.status) }}</span>
          <span class="rtime muted">{{ timeText(t.createdAt) }}</span>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.home {
  display: flex;
  flex-direction: column;
  gap: 26px;
  max-width: 880px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 12px;
}
.head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
}
.more {
  color: var(--aa-primary);
  font-size: 0.85rem;
}
.self {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 18px 20px;
}
.self .icon {
  font-size: 2.2rem;
}
.self .name {
  font-weight: 700;
  font-size: 1.1rem;
}
.self .status {
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  display: flex;
  align-items: center;
  gap: 6px;
}

.caps {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 12px;
}
.cap {
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  transition: border-color 0.15s;
}
.cap:hover {
  border-color: var(--aa-primary);
}
.cap-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.cap-ic {
  font-size: 1.5rem;
}
.cap-pill {
  font-size: 0.68rem;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 999px;
}
.cap-pill.ok {
  color: var(--aa-success);
  background: color-mix(in srgb, var(--aa-success) 14%, transparent);
}
.cap-pill.soon {
  color: #9a6a00;
  background: #ffedcc;
}
@media (prefers-color-scheme: dark) {
  .cap-pill.soon {
    color: #ffce80;
    background: #4a3a16;
  }
}
.cap-nm {
  font-weight: 700;
  font-size: 1rem;
}
.cap-ds {
  font-size: 0.8rem;
  line-height: 1.5;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: 14px;
}
.empty {
  padding: 30px;
  text-align: center;
}
.empty p {
  margin: 4px 0;
}
.recent {
  list-style: none;
  margin: 0;
  padding: 6px 0;
}
.recent li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 18px;
  font-size: 0.88rem;
}
.recent li + li {
  border-top: 1px solid var(--aa-border);
}
.rname {
  font-weight: 600;
}
.rfiles {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rtime {
  font-variant-numeric: tabular-nums;
}
</style>
