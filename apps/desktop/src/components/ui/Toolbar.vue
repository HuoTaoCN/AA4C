<script setup lang="ts">
// 页面顶部标题行 + 右侧操作（UI_DESIGN_SPEC §5.2）。
defineProps<{ title: string; subtitle?: string }>();
</script>

<template>
  <header class="ui-toolbar">
    <div class="text">
      <h1>{{ title }}</h1>
      <p v-if="subtitle" class="sub">{{ subtitle }}</p>
    </div>
    <div v-if="$slots.actions" class="actions"><slot name="actions" /></div>
  </header>
</template>

<style scoped>
.ui-toolbar {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--sp-4);
  margin-bottom: var(--sp-5);
}
h1 {
  margin: 0;
  font-size: var(--fs-xl);
  font-weight: var(--fw-bold);
}
.sub {
  margin: var(--sp-1) 0 0;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.actions {
  display: flex;
  gap: var(--sp-2);
  flex: none;
}
</style>
