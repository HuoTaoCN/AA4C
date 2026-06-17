<script setup lang="ts">
import { computed, ref } from "vue";
import SyncNode from "../components/SyncNode.vue";
import {
  SAMPLE_TREE,
  STATUS_LEGEND,
  pruneTree,
  type SyncFile,
} from "../lib/sync-preview";

type Filter = "all" | "online" | "local";
const filter = ref<Filter>("all");

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "online", label: "可下载" },
  { key: "local", label: "本地有" },
];

function keep(f: SyncFile): boolean {
  if (filter.value === "online") return f.status === "online";
  if (filter.value === "local") return f.status === "local";
  return true;
}

const tree = computed(() => pruneTree(SAMPLE_TREE, keep));
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

    <!-- 目录树 -->
    <div v-if="tree.length" class="tree card">
      <SyncNode v-for="(n, i) in tree" :key="i" :node="n" :depth="0" />
    </div>
    <div v-else class="empty card muted">该筛选下没有文件。</div>
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
  margin-bottom: 14px;
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
.legend .dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}
.legend .dot.local {
  background: var(--aa-success);
}
.legend .dot.online {
  background: #e0a400;
}
.legend .dot.offline {
  background: var(--aa-danger);
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
.tree {
  padding: 2px 0;
  overflow: hidden;
}
.empty {
  padding: 24px;
  text-align: center;
}
</style>
