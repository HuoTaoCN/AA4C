<script setup lang="ts">
import { computed } from "vue";
import { useTransferStore } from "../stores/transfer";
import TransferCard from "./TransferCard.vue";

const transfer = useTransferStore();

// 最活跃的一条：优先传输中，否则取第一条
const top = computed(() => {
  const list = transfer.activeList;
  return list.find((t) => t.status === "transferring") ?? list[0] ?? null;
});
const others = computed(() => transfer.activeList.length - 1);
</script>

<template>
  <div class="taskbar" v-if="top">
    <TransferCard :task="top" />
    <span v-if="others > 0" class="more">还有 {{ others }} 个</span>
  </div>
</template>

<style scoped>
.taskbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 18px;
  border-top: 1px solid var(--aa-border);
  background: var(--aa-surface);
}
.more {
  font-size: 0.78rem;
  color: var(--aa-text-dim);
  white-space: nowrap;
}
</style>
