<script setup lang="ts">
import { computed, ref } from "vue";
import { useDeviceStore } from "../stores/devices";
import { useSettingsStore } from "../stores/settings";
import { useTransferStore } from "../stores/transfer";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { humanBytes } from "../lib/format";

const devices = useDeviceStore();
const settings = useSettingsStore();
const transfer = useTransferStore();
const toast = useToastStore();

const overrideDir = ref<string | null>(null);

const task = computed(() => transfer.incomingList[0] ?? null);
const peer = computed(() => (task.value ? devices.nameOf(task.value.peer) : ""));
const totalSize = computed(() =>
  task.value ? humanBytes(task.value.totalBytes) : "",
);
const saveDir = computed(
  () => overrideDir.value ?? settings.settings?.saveDir ?? "默认接收目录",
);

async function changeDir() {
  const picked = await settings.pickSaveDir();
  if (picked) overrideDir.value = picked;
}

async function respond(accept: boolean) {
  if (!task.value) return;
  const id = task.value.id;
  const dir = overrideDir.value ?? undefined;
  overrideDir.value = null;
  try {
    await transfer.accept(id, accept, dir);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div v-if="task" class="overlay">
    <div class="dialog card">
      <h3>收到文件请求</h3>
      <p class="sub">
        <b>{{ peer }}</b> 想把
        <b>{{ task.files.length }} 个文件（{{ totalSize }}）</b> AA 给你
      </p>
      <div class="dir">
        保存到：<span class="path">{{ saveDir }}</span>
        <button class="link" @click="changeDir">更改</button>
      </div>
      <div class="actions">
        <button class="btn btn-ghost" @click="respond(false)">拒绝</button>
        <button class="btn btn-primary" @click="respond(true)">接收</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 60;
}
.dialog {
  width: 380px;
  padding: var(--sp-5);
}
h3 {
  margin: 0 0 var(--sp-2);
}
.sub {
  margin: 0 0 var(--sp-3);
  font-size: var(--fs-base);
}
.dir {
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
  margin-bottom: var(--sp-4);
  word-break: break-all;
}
.path {
  color: var(--aa-text);
}
.link {
  color: var(--aa-primary);
  font-weight: var(--fw-bold);
  margin-left: var(--sp-1);
}
.actions {
  display: flex;
  gap: var(--sp-2);
}
.actions .btn {
  flex: 1;
}
</style>
