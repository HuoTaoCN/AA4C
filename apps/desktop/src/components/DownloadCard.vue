<script setup lang="ts">
import { computed } from "vue";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useDownloadStore, type LiveDownloadTask } from "../stores/download";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { downloadStatusText, errorText, etaText, humanBytes, humanSpeed, taskTitle } from "../lib/format";

const props = defineProps<{ task: LiveDownloadTask }>();
const download = useDownloadStore();
const toast = useToastStore();

const percent = computed(() =>
  props.task.totalBytes > 0
    ? Math.min(100, Math.round((props.task.downloadedBytes / props.task.totalBytes) * 100))
    : 0,
);
const title = computed(() => taskTitle(props.task.url, props.task.id));
const showBtStats = computed(
  () =>
    props.task.kind === "bt" &&
    (props.task.status === "active" || props.task.status === "waiting") &&
    props.task.seeders !== undefined,
);
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
/** 取消并删除本地文件——不可撤销，二次确认（对标 FDM/Motrix 的"删除任务和文件"）。 */
async function cancelAndDelete() {
  if (!window.confirm(`确定要取消「${title.value}」并删除已下载的本地文件吗？此操作无法撤销。`)) {
    return;
  }
  try {
    await download.cancel(props.task.id, true);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function retry() {
  try {
    await download.retry(props.task.id);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
/** 复制原始下载链接（对标 Motrix 的"复制下载地址"）——重新添加、换个工具下、
 *  发给别人都要用到。 */
async function copyLink() {
  try {
    await writeText(props.task.url);
    toast.push("success", "链接已复制");
  } catch {
    toast.push("error", "复制失败");
  }
}
/** 用默认程序直接打开文件本身。 */
async function openFile() {
  if (props.task.savePath) await openPath(props.task.savePath);
}
/** 在文件管理器里定位到这个文件（区别于 openFile：不是打开文件内容，是打开
 *  它所在的文件夹并选中它）——此前误用 `openPath` 实现"打开所在文件夹"，
 *  实际效果是直接打开文件本身，标签和行为对不上，这里一并修正。 */
async function openFolder() {
  if (props.task.savePath) await revealItemInDir(props.task.savePath);
}
</script>

<template>
  <div class="dc">
    <div class="row">
      <span class="title" :title="task.url">{{ title }}</span>
      <span class="actions">
        <button v-if="task.status === 'active'" title="暂停" @click="pause">⏸</button>
        <button v-if="task.status === 'paused'" title="继续" @click="resume">▶</button>
        <button v-if="task.status === 'error'" title="重试" @click="retry">↻</button>
        <button title="复制下载链接" @click="copyLink">🔗</button>
        <button
          v-if="task.status !== 'complete' && task.status !== 'removed'"
          class="danger"
          title="取消"
          @click="cancel"
        >
          ✕
        </button>
        <button
          v-if="task.status !== 'complete' && task.status !== 'removed'"
          class="danger"
          title="取消并删除文件"
          @click="cancelAndDelete"
        >
          🗑
        </button>
        <button v-if="task.status === 'complete'" title="打开文件" @click="openFile">📄</button>
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
        <template v-if="showBtStats">
          <span>{{ task.seeders }} 做种</span>
          <span>{{ task.peers }} 连接</span>
          <span v-if="task.ratio !== undefined">分享率 {{ task.ratio.toFixed(2) }}</span>
        </template>
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
