<script setup lang="ts">
import { openPath } from "@tauri-apps/plugin-opener";
import { useToastStore, type Toast } from "../stores/toast";

const toast = useToastStore();

async function onClick(t: Toast) {
  if (t.openDir) {
    try {
      await openPath(t.openDir);
    } catch {
      /* 打开失败时忽略，toast 仍会自动消失 */
    }
  }
  toast.dismiss(t.id);
}
</script>

<template>
  <div class="toast-host">
    <div
      v-for="t in toast.items"
      :key="t.id"
      class="toast"
      :class="t.kind"
      @click="onClick(t)"
    >
      <span class="msg">{{ t.message }}</span>
      <span v-if="t.openDir" class="hint">点击打开文件夹</span>
    </div>
  </div>
</template>

<style scoped>
.toast-host {
  position: fixed;
  top: 64px;
  right: 18px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  z-index: 50;
}
.toast {
  max-width: 320px;
  padding: 11px 15px;
  border-radius: var(--aa-radius-sm);
  background: var(--aa-surface);
  border: 1px solid var(--aa-border);
  box-shadow: 0 6px 22px rgba(0, 0, 0, 0.14);
  cursor: pointer;
  font-size: 0.9rem;
}
.toast.success {
  border-left: 3px solid var(--aa-success);
}
.toast.error {
  border-left: 3px solid var(--aa-danger);
}
.toast.info {
  border-left: 3px solid var(--aa-primary);
}
.hint {
  display: block;
  margin-top: 3px;
  font-size: 0.75rem;
  color: var(--aa-text-dim);
}
</style>
