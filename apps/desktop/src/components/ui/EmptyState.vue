<script setup lang="ts">
// 空状态（UI_DESIGN_SPEC §5.2 / §6）。
//
// 规则：**一句话说清楚现在是什么状况，再一句话说下一步做什么。**
// 只写「暂无数据」是把问题丢回给用户。
defineProps<{
  title: string;
  /** 下一步。没有真正的下一步时才留空。 */
  hint?: string;
}>();
</script>

<template>
  <div class="ui-empty">
    <p class="title">{{ title }}</p>
    <p v-if="hint" class="hint">{{ hint }}</p>
    <div v-if="$slots.action" class="action"><slot name="action" /></div>
  </div>
</template>

<style scoped>
.ui-empty {
  background: var(--aa-surface);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius);
  padding: var(--sp-6) var(--sp-5);
  text-align: center;
}
.title {
  margin: 0;
  font-size: var(--fs-base);
}
.hint {
  margin: var(--sp-2) 0 0;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.action {
  margin-top: var(--sp-4);
}
</style>
