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

async function cancel() {
  try {
    await transfer.cancel(props.task.id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="tc">
    <div class="row">
      <span class="title">{{ title }}</span>
      <button class="cancel" title="取消" @click="cancel">✕</button>
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
  justify-content: space-between;
}
.title {
  font-weight: 600;
  font-size: 0.9rem;
}
.cancel {
  color: var(--aa-text-dim);
  font-size: 0.9rem;
  padding: 2px 6px;
}
.cancel:hover {
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
