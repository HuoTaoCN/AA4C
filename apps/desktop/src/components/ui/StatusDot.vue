<script setup lang="ts">
// 状态圆点 + 文案（UI_DESIGN_SPEC §5.2）。
withDefaults(
  defineProps<{
    tone?: "ok" | "warn" | "danger" | "muted";
    /** 文案。留空则只画点。 */
    label?: string;
  }>(),
  { tone: "muted" },
);
</script>

<template>
  <span class="ui-status" :class="`t-${tone}`">
    <span class="dot" />
    <span v-if="label">{{ label }}</span>
  </span>
</template>

<style scoped>
.ui-status {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
  flex: none;
}
.t-ok {
  color: var(--aa-success);
}
.t-warn {
  color: var(--aa-warn);
}
.t-danger {
  color: var(--aa-danger);
}
.t-muted {
  color: var(--aa-text-dim);
}
</style>
