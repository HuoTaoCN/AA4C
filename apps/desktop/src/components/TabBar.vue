<script setup lang="ts">
defineProps<{
  tabs: { key: string; label: string }[];
  modelValue: string;
}>();
defineEmits<{ "update:modelValue": [value: string] }>();
</script>

<template>
  <div class="tab-bar" role="tablist">
    <button
      v-for="tab in tabs"
      :key="tab.key"
      type="button"
      role="tab"
      :aria-selected="modelValue === tab.key"
      class="tab-btn"
      :class="{ active: modelValue === tab.key }"
      @click="$emit('update:modelValue', tab.key)"
    >
      {{ tab.label }}
    </button>
  </div>
</template>

<style scoped>
.tab-bar {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  background: var(--aa-surface-2);
  border-radius: var(--aa-radius-sm);
  margin-bottom: 18px;
}
.tab-btn {
  padding: 6px 14px;
  border: none;
  background: transparent;
  color: var(--aa-text-dim);
  font-size: 0.82rem;
  font-weight: 500;
  border-radius: calc(var(--aa-radius-sm) - 3px);
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s;
  white-space: nowrap;
}
.tab-btn:hover {
  color: var(--aa-text);
}
.tab-btn.active {
  background: var(--aa-surface);
  color: var(--aa-text);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
}
</style>
