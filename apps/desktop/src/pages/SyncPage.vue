<script setup lang="ts">
import { computed, ref } from "vue";
import { useToastStore } from "../stores/toast";
import { humanBytes } from "../lib/format";
import {
  SAMPLE_GROUPS,
  STATUS_LEGEND,
  type SyncEntry,
  type SyncStatus,
} from "../lib/sync-preview";

const toast = useToastStore();

type Filter = "all" | "online" | "local";
const filter = ref<Filter>("all");

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "online", label: "可下载" },
  { key: "local", label: "本地有" },
];

function keep(e: SyncEntry): boolean {
  if (filter.value === "online") return e.status === "online";
  if (filter.value === "local") return e.status === "local";
  return true;
}

const groups = computed(() =>
  SAMPLE_GROUPS.map((g) => ({ title: g.title, entries: g.entries.filter(keep) })).filter(
    (g) => g.entries.length > 0,
  ),
);

function statusLabel(s: SyncStatus): string {
  return STATUS_LEGEND.find((x) => x.status === s)!.label;
}

function onClick(e: SyncEntry) {
  switch (e.status) {
    case "online":
      toast.push("info", `演示：将从「${e.owner}」取回 ${e.name}（同步功能 V0.2 上线）`);
      break;
    case "offline":
      toast.push("info", `「${e.owner}」当前离线，等它上线后即可取回`);
      break;
    case "local":
      toast.push("info", `${e.name} 已在本机`);
      break;
  }
}
</script>

<template>
  <div class="sync">
    <div class="head">
      <h2>同步</h2>
      <span class="preview">预览 · 跨设备同步将在 V0.2 上线</span>
    </div>

    <p class="intro muted">
      把你「自己的设备」连成一个文件空间：在哪台设备上有、能不能直接拿到，一眼可见。
      下面是设计预览（示例文件）。
    </p>

    <!-- 图例 + 筛选 -->
    <div class="bar card">
      <div class="legend">
        <span v-for="l in STATUS_LEGEND" :key="l.status" class="leg">
          <span class="dot" :class="l.status"></span>{{ l.label }}
        </span>
      </div>
      <div class="filters">
        <button
          v-for="f in FILTERS"
          :key="f.key"
          class="ftab"
          :class="{ on: filter === f.key }"
          @click="filter = f.key"
        >
          {{ f.label }}
        </button>
      </div>
    </div>

    <!-- 统一文件视图 -->
    <section v-for="g in groups" :key="g.title" class="group">
      <h3>{{ g.title }}</h3>
      <ul class="card">
        <li v-for="e in g.entries" :key="g.title + e.name" @click="onClick(e)">
          <span class="dot" :class="e.status" :title="statusLabel(e.status)"></span>
          <span class="fn">{{ e.name }}</span>
          <span class="sz muted">{{ humanBytes(e.size) }}</span>
          <span class="ow muted">{{ e.owner }}</span>
        </li>
      </ul>
    </section>
    <div v-if="groups.length === 0" class="empty card muted">该筛选下没有文件。</div>
  </div>
</template>

<style scoped>
.sync {
  max-width: 820px;
}
.head {
  display: flex;
  align-items: baseline;
  gap: 12px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 6px;
}
.preview {
  font-size: 0.72rem;
  font-weight: 600;
  color: #9a6a00;
  background: #ffedcc;
  padding: 2px 9px;
  border-radius: 999px;
}
@media (prefers-color-scheme: dark) {
  .preview {
    color: #ffce80;
    background: #4a3a16;
  }
}
.intro {
  font-size: 0.85rem;
  line-height: 1.6;
  margin: 0 0 16px;
}

.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  margin-bottom: 18px;
  flex-wrap: wrap;
}
.legend {
  display: flex;
  gap: 16px;
}
.leg {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 0.82rem;
  color: var(--aa-text-dim);
}
.filters {
  display: flex;
  gap: 4px;
}
.ftab {
  font-size: 0.82rem;
  padding: 5px 12px;
  border-radius: 999px;
  color: var(--aa-text-dim);
}
.ftab.on {
  background: var(--aa-primary-dim);
  color: var(--aa-primary);
  font-weight: 600;
}

.group h3 {
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  margin: 16px 0 8px;
}
ul {
  list-style: none;
  margin: 0;
  padding: 4px 0;
}
li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  cursor: pointer;
}
li:hover {
  background: var(--aa-surface-2);
}
li + li {
  border-top: 1px solid var(--aa-border);
}
.fn {
  flex: 1;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.sz {
  font-size: 0.8rem;
  font-variant-numeric: tabular-nums;
}
.ow {
  font-size: 0.8rem;
  min-width: 88px;
  text-align: right;
}

/* 状态点 */
.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
  display: inline-block;
}
.dot.local {
  background: var(--aa-success);
}
.dot.online {
  background: #e0a400;
}
.dot.offline {
  background: var(--aa-danger);
}
.empty {
  padding: 24px;
  text-align: center;
}

@media (max-width: 700px) {
  .ow {
    display: none;
  }
}
</style>
