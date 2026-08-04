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
/* TransferCard 根节点默认按内容宽度撑开（flex item 不吃 width:100%），标题的
   text-overflow:ellipsis 因此没有真正的宽度上限可截断——长设备名/长文件名会把
   整条任务条顶宽，900px 最小窗口下可能出现横向滚动（UI_DESIGN_SPEC.md §8 明确
   禁止）。给够 flex-grow + min-width:0 让它老老实实收缩、把截断交还给内部样式。 */
.taskbar :deep(.tc) {
  flex: 1;
  min-width: 0;
}
.more {
  font-size: 0.78rem;
  color: var(--aa-text-dim);
  white-space: nowrap;
}
</style>
