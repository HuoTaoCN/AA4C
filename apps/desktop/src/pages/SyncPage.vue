<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import SyncNode from "../components/SyncNode.vue";
import { useSyncStore } from "../stores/sync";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { STATUS_LEGEND, buildTree, pruneTree, type SyncFile } from "../lib/sync-tree";

const sync = useSyncStore();
const toast = useToastStore();

onMounted(() => {
  void sync.load();
});

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

const tree = computed(() => pruneTree(buildTree(sync.scopes, sync.files), keep));
const folders = computed(() => sync.scopes.filter((s) => s.kind === "folder"));
const rescanning = ref(false);

async function addFolder() {
  try {
    const scope = await sync.addFolder();
    if (scope) toast.push("success", "已添加同步文件夹，正在扫描…");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function removeFolder(id: string) {
  try {
    await sync.removeScope(id);
    toast.push("info", "已移除该同步文件夹");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function rescan() {
  rescanning.value = true;
  try {
    await sync.rescan();
    toast.push("info", "已重新扫描");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  } finally {
    rescanning.value = false;
  }
}
</script>

<template>
  <div class="sync">
    <div class="head">
      <h2>同步</h2>
      <button class="btn btn-ghost small" :disabled="rescanning" @click="rescan">
        {{ rescanning ? "扫描中…" : "重新扫描" }}
      </button>
    </div>

    <p class="intro muted">
      把你「自己的设备」连成一个文件空间：在哪台设备上有、能不能直接拿到，一眼可见。
      跨设备拉取（黄色「可下载」）正在路上，当前先看本机文件 + 「收到的」。
    </p>

    <!-- 同步范围管理 -->
    <div class="card scopes">
      <div class="srow" v-for="s in folders" :key="s.id">
        <span class="fic">📁</span>
        <span class="nm">{{ s.localPath }}</span>
        <button class="btn btn-ghost small" @click="removeFolder(s.id)">移除</button>
      </div>
      <div class="srow add">
        <button class="btn btn-ghost small" @click="addFolder">+ 添加同步文件夹</button>
        <span class="hint muted">「收到的」自动纳入同步，无需添加</span>
      </div>
    </div>

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
    <div v-else class="empty card muted">
      {{ sync.files.length ? "该筛选下没有文件。" : "还没有文件：添加一个同步文件夹，或者等收到文件后自动出现在「收到的」里。" }}
    </div>
  </div>
</template>

<style scoped>
.sync {
  max-width: 820px;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
h2 {
  font-size: 1rem;
  margin: 0;
}
.small {
  padding: 5px 12px;
  min-height: 32px;
  font-size: 0.8rem;
}
.intro {
  font-size: 0.85rem;
  line-height: 1.6;
  margin: 6px 0 16px;
}
.scopes {
  padding: 4px 0;
  margin-bottom: 14px;
}
.srow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 14px;
}
.srow + .srow {
  border-top: 1px solid var(--aa-border);
}
.srow .fic {
  font-size: 0.95rem;
}
.srow .nm {
  flex: 1;
  font-size: 0.85rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.srow.add {
  justify-content: space-between;
}
.srow .hint {
  font-size: 0.76rem;
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
