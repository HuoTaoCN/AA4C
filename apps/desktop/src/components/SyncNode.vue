<script setup lang="ts">
import { ref } from "vue";
import { useToastStore } from "../stores/toast";
import { useSyncStore } from "../stores/sync";
import { asCommandError } from "../lib/api";
import { humanBytes } from "../lib/format";
import { statusLabel, type SyncNode } from "../lib/sync-tree";

const props = defineProps<{ node: SyncNode; depth: number }>();
defineOptions({ name: "SyncNode" });

const toast = useToastStore();
const sync = useSyncStore();
const open = ref(props.depth < 1); // 顶层目录默认展开
const fetching = ref(false);

const indent = (d: number) => ({ paddingLeft: `${12 + d * 18}px` });

async function onFile() {
  if (props.node.kind !== "file") return;
  const f = props.node;
  if (f.status === "online") {
    if (fetching.value) return;
    fetching.value = true;
    toast.push("info", `正在从「${f.owners[0]}」取回 ${f.name}…`);
    try {
      await sync.fetch(f.basePath, f.hash);
      // 内容拉到本机后扫描会自动把它转绿（事件驱动刷新），无需手动操作
      toast.push("success", `${f.name} 取回中，完成后会自动转为「本地有」`);
    } catch (e) {
      toast.push("error", asCommandError(e).message);
    } finally {
      fetching.value = false;
    }
  } else if (f.status === "offline") {
    toast.push("info", `「${f.owners[0]}」当前离线，等它上线后即可取回`);
  } else {
    toast.push("info", `${f.name} 已在本机`);
  }
}
</script>

<template>
  <!-- 目录：可展开 -->
  <template v-if="node.kind === 'dir'">
    <div class="row dir" :style="indent(depth)" @click="open = !open">
      <span class="caret" :class="{ open }">▸</span>
      <span class="fic">📁</span>
      <span class="nm">{{ node.name }}</span>
      <span class="cnt muted">{{ node.children.length }}</span>
    </div>
    <template v-if="open">
      <SyncNode
        v-for="(c, i) in node.children"
        :key="i"
        :node="c"
        :depth="depth + 1"
      />
    </template>
  </template>

  <!-- 文件：状态点 + 文字标签 -->
  <div v-else class="row file" :style="indent(depth)" @click="onFile">
    <span class="fic">📄</span>
    <span class="nm">{{ node.name }}</span>
    <span v-if="node.conflict" class="conflict" title="这个名字有多个不同版本，各自独立取回">
      多版本
    </span>
    <span class="pill" :class="node.status">
      <span class="dot"></span>{{ statusLabel(node.status) }}
    </span>
    <span class="sz muted">{{ humanBytes(node.size) }}</span>
    <span class="ow muted">{{ node.owners.join("、") }}</span>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 8px 14px 8px 12px;
  cursor: pointer;
}
.row:hover {
  background: var(--aa-surface-2);
}
.row + .row {
  border-top: 1px solid var(--aa-border);
}
.caret {
  display: inline-block;
  width: 12px;
  font-size: 0.7rem;
  color: var(--aa-text-dim);
  transition: transform 0.12s;
}
.caret.open {
  transform: rotate(90deg);
}
.fic {
  font-size: 1rem;
}
.dir .nm {
  font-weight: 600;
}
.nm {
  flex: 1;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cnt {
  font-size: 0.78rem;
}
.sz {
  font-size: 0.8rem;
  font-variant-numeric: tabular-nums;
}
.ow {
  font-size: 0.8rem;
  min-width: 96px;
  text-align: right;
}

/* 冲突（多版本）标记 */
.conflict {
  font-size: 0.68rem;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: 999px;
  color: #7a5200;
  background: #fff0d6;
  white-space: nowrap;
}
@media (prefers-color-scheme: dark) {
  .conflict {
    color: #ffd591;
    background: #4a3a16;
  }
}

/* 状态标签：彩点 + 文字 */
.pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 0.72rem;
  font-weight: 600;
  padding: 2px 9px;
  border-radius: 999px;
}
.pill .dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
}
.pill.local {
  color: var(--aa-success);
  background: color-mix(in srgb, var(--aa-success) 14%, transparent);
}
.pill.online {
  color: #9a6a00;
  background: #ffedcc;
}
.pill.offline {
  color: var(--aa-danger);
  background: color-mix(in srgb, var(--aa-danger) 14%, transparent);
}
@media (prefers-color-scheme: dark) {
  .pill.online {
    color: #ffce80;
    background: #4a3a16;
  }
}
@media (max-width: 700px) {
  .ow {
    display: none;
  }
}
</style>
