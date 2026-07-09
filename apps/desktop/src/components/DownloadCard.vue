<script setup lang="ts">
import { computed } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import { useDownloadStore, type LiveDownloadTask } from "../stores/download";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { baseName, downloadStatusText, errorText, etaText, humanBytes, humanSpeed } from "../lib/format";

const props = defineProps<{ task: LiveDownloadTask }>();
const download = useDownloadStore();
const toast = useToastStore();

const percent = computed(() =>
  props.task.totalBytes > 0
    ? Math.min(100, Math.round((props.task.downloadedBytes / props.task.totalBytes) * 100))
    : 0,
);
const title = computed(() => baseName(props.task.url));
const eta = computed(() =>
  props.task.status === "active"
    ? etaText(props.task.totalBytes - props.task.downloadedBytes, props.task.speedBps)
    : "",
);

async function pause() {
  try {
    await download.pause(props.task.id);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function resume() {
  try {
    await download.resume(props.task.id);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function cancel() {
  try {
    await download.cancel(props.task.id);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function openFolder() {
  if (props.task.savePath) await openPath(props.task.savePath);
}
</script>

<template>
  <div class="dc">
    <div class="row">
      <span class="title" :title="task.url">{{ title }}</span>
      <span class="actions">
        <button v-if="task.status === 'active'" title="暂停" @click="pause">⏸</button>
        <button v-if="task.status === 'paused'" title="继续" @click="resume">▶</button>
        <button
          v-if="task.status !== 'complete' && task.status !== 'removed'"
          class="danger"
          title="取消"
          @click="cancel"
        >
          ✕
        </button>
        <button v-if="task.status === 'complete'" title="打开所在文件夹" @click="openFolder">
          📂
        </button>
      </span>
    </div>
    <div class="bar"><i :style="{ width: percent + '%' }" :class="{ error: task.status === 'error' }"></i></div>
    <div class="meta">
      <span>{{ downloadStatusText(task.status) }}</span>
      <template v-if="task.status === 'active' || task.status === 'paused'">
        <span>{{ percent }}%</span>
        <span v-if="task.totalBytes > 0">
          {{ humanBytes(task.downloadedBytes) }} / {{ humanBytes(task.totalBytes) }}
        </span>
        <span v-if="task.status === 'active' && task.speedBps > 0">{{ humanSpeed(task.speedBps) }}</span>
        <span v-if="eta">{{ eta }}</span>
      </template>
      <span v-if="task.status === 'error' && task.error" class="err">{{ task.error }}</span>
    </div>
  </div>
</template>

<style scoped>
.dc {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 100%;
}
.row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.title {
  flex: 1;
  font-weight: 600;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}
.actions button {
  color: var(--aa-text-dim);
  font-size: 0.9rem;
  padding: 2px 6px;
}
.actions button:hover {
  color: var(--aa-primary);
}
.actions button.danger:hover {
  color: var(--aa-danger);
}
.bar {
  height: 7px;
  border-radius: 4px;
  background: var(--aa-surface-2);
  overflow: hidden;
}
.bar i {
  display: block;
  height: 100%;
  background: var(--aa-primary);
  transition: width 0.2s;
}
.bar i.error {
  background: var(--aa-danger);
  width: 100% !important;
}
.meta {
  display: flex;
  gap: 12px;
  font-size: 0.78rem;
  color: var(--aa-text-dim);
}
.err {
  color: var(--aa-danger);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
