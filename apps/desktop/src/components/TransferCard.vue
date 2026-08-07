<script setup lang="ts">
import { computed } from "vue";
import { useDeviceStore } from "../stores/devices";
import { useTransferStore, type ActiveTask } from "../stores/transfer";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { etaText, humanSpeed, statusText } from "../lib/format";

const props = defineProps<{ task: ActiveTask }>();
const devices = useDeviceStore();
const transfer = useTransferStore();
const toast = useToastStore();

const percent = computed(() =>
  props.task.totalBytes > 0
    ? Math.min(100, Math.round((props.task.transferredBytes / props.task.totalBytes) * 100))
    : 0,
);
const peer = computed(() => devices.nameOf(props.task.peer));
const arrow = computed(() => (props.task.direction === "send" ? "⬆" : "⬇"));
const title = computed(
  () => `${arrow.value} ${props.task.direction === "send" ? "发送至" : "接收自"} ${peer.value}`,
);
const eta = computed(() =>
  props.task.status === "transferring"
    ? etaText(props.task.totalBytes - props.task.transferredBytes, props.task.speedBps)
    : "",
);
// 连接质量（里程碑 C4/C5）：只有发起方收得到 via 事件，接收一方本地任务上永远是
// undefined，这时不显示徽标——不确定就不瞎猜，比显示错误的档位更诚实。
// `punch`（打洞后升级成的直连）在展示上并入「直连」，不单独暴露成第三个词——
// 打洞只是"怎么找到对方"的手段，一旦连上就是货真价实的直连，用户不需要关心过程
// （见 CONNECT_DESIGN.md §10）。
const viaText = computed(() => {
  switch (props.task.via) {
    case "relay":
      return "中继（较慢）";
    case "direct":
    case "punch":
      return "直连";
    default:
      return "";
  }
});

async function cancel() {
  try {
    await transfer.cancel(props.task.id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

// 暂停/继续只对**本机发起的发送**有意义：接收方向没有"我这边继续"的说法，
// 要由发送方重新发起（后端也会拒，这里就不摆一个点了必然报错的按钮）。
const canPause = computed(
  () => props.task.direction === "send" && props.task.status === "transferring",
);
const canResume = computed(
  () => props.task.direction === "send" && props.task.status === "paused",
);

async function pause() {
  try {
    await transfer.pause(props.task.id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
async function resume() {
  try {
    await transfer.resume(props.task.id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="tc">
    <div class="row">
      <span class="title">{{ title }}</span>
      <span v-if="viaText" class="via" :class="{ relay: task.via === 'relay' }">{{
        viaText
      }}</span>
      <button v-if="canPause" class="act" title="暂停" @click="pause">⏸</button>
      <button v-if="canResume" class="act" title="继续" @click="resume">▶</button>
      <button class="act danger" title="取消" @click="cancel">✕</button>
    </div>
    <div class="file">
      {{ task.currentFile || statusText(task.status) }}
    </div>
    <div class="bar"><i :style="{ width: percent + '%' }"></i></div>
    <div class="meta">
      <span>{{ percent }}%</span>
      <span v-if="task.status === 'transferring' && task.speedBps > 0">
        {{ humanSpeed(task.speedBps) }}
      </span>
      <span v-if="eta">{{ eta }}</span>
      <span v-if="task.status === 'waiting_accept'">等待对方确认…</span>
      <span v-if="task.status === 'paused'">已暂停，点 ▶ 接着传</span>
    </div>
  </div>
</template>

<style scoped>
.tc {
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
.via {
  flex-shrink: 0;
  font-size: 0.72rem;
  font-weight: 600;
  color: var(--aa-text-dim);
  background: var(--aa-surface-2);
  padding: 2px 8px;
  border-radius: 999px;
}
.via.relay {
  color: #9a6a00;
  background: #ffedcc;
}
@media (prefers-color-scheme: dark) {
  .via.relay {
    color: #ffce80;
    background: #4a3a16;
  }
}
.act {
  color: var(--aa-text-dim);
  font-size: 0.9rem;
  padding: 2px 6px;
  flex-shrink: 0;
}
.act:hover {
  color: var(--aa-primary);
}
.act.danger:hover {
  color: var(--aa-danger);
}
.file {
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
.meta {
  display: flex;
  gap: 12px;
  font-size: 0.78rem;
  color: var(--aa-text-dim);
}
</style>
