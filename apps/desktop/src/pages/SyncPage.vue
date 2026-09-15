<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import SyncNode from "../components/SyncNode.vue";
import { useSyncStore } from "../stores/sync";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { STATUS_LEGEND, buildTree, pruneTree, type SyncFile } from "../lib/sync-tree";
import { Button, Card, EmptyState, Icon, Toolbar } from "../components/ui";

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

const tree = computed(() => pruneTree(buildTree(sync.files), keep));
const folders = computed(() => sync.scopes.filter((s) => s.kind === "folder"));
const rescanning = ref(false);
const refreshing = ref(false);

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

async function refresh() {
  refreshing.value = true;
  try {
    await sync.refresh();
    toast.push("info", "已和在线设备同步文件清单");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  } finally {
    refreshing.value = false;
  }
}
</script>

<template>
  <div class="sync">
    <Toolbar
      title="同步"
      subtitle="把你「自己的设备」连成一个文件空间：在哪台设备上有、能不能直接拿到，一眼可见"
    >
      <template #actions>
        <Button size="sm" variant="ghost" :disabled="refreshing" @click="refresh">
          <Icon name="refresh" :size="14" />
          {{ refreshing ? "刷新中…" : "刷新设备" }}
        </Button>
        <Button size="sm" variant="ghost" :disabled="rescanning" @click="rescan">
          {{ rescanning ? "扫描中…" : "重新扫描" }}
        </Button>
      </template>
    </Toolbar>

    <!-- 同步范围管理 -->
    <Card padding="none">
      <div v-for="s in folders" :key="s.id" class="srow">
        <span class="fic">📁</span>
        <span class="nm">{{ s.localPath }}</span>
        <Button size="sm" variant="ghost" @click="removeFolder(s.id)">移除</Button>
      </div>
      <div class="srow">
        <Button size="sm" variant="ghost" @click="addFolder">
          <Icon name="plus" :size="14" /> 添加同步文件夹
        </Button>
        <span class="hint">「收到的」自动纳入同步，无需添加</span>
      </div>
    </Card>

    <!-- 图例 + 筛选 -->
    <Card padding="sm">
      <div class="bar">
        <div class="legend">
          <span v-for="l in STATUS_LEGEND" :key="l.status" class="leg">
            <span class="dot" :class="l.status" />{{ l.label }}
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
    </Card>

    <!-- 目录树 -->
    <Card v-if="tree.length" padding="sm">
      <SyncNode v-for="(n, i) in tree" :key="i" :node="n" :depth="0" />
    </Card>
    <EmptyState
      v-else-if="sync.files.length"
      title="该筛选下没有文件。"
      hint="换一个筛选条件看看。"
    />
    <EmptyState
      v-else
      title="还没有文件。"
      hint="添加一个同步文件夹，或者等收到文件后自动出现在「收到的」里。只有标为「我的设备」的设备才会互通文件清单。"
    />

    <p class="foot">
      点黄色「可下载」文件即可从在线设备取回到本机，完成后自动转为绿色「本地有」。
    </p>
  </div>
</template>

<style scoped>
.sync {
  max-width: 820px;
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.srow {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-2) var(--sp-4);
}
.srow + .srow {
  border-top: 1px solid var(--aa-border);
}
.fic {
  flex: none;
}
.nm {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
}
.hint,
.foot {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-4);
  flex-wrap: wrap;
}
.legend {
  display: flex;
  gap: var(--sp-4);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.leg {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--aa-text-dim);
}
.dot.local {
  background: var(--aa-success);
}
.dot.online {
  background: var(--aa-warn);
}
.dot.offline {
  background: var(--aa-danger);
}
.filters {
  display: flex;
  gap: var(--sp-1);
}
.ftab {
  padding: var(--sp-1) var(--sp-3);
  border-radius: var(--aa-radius-sm);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.ftab.on {
  background: var(--aa-primary-dim);
  color: var(--aa-primary);
}
</style>
