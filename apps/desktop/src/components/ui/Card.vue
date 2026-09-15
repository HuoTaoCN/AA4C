<script setup lang="ts">
// 全应用唯一的卡片实现（UI_DESIGN_SPEC §5.2）。
// F3 之前每个页面各写一遍 `.card`，圆角、边框、内边距各不相同。
withDefaults(
  defineProps<{
    /** 内边距档位。`none` 给自带内边距的内容（列表、表格）用。 */
    padding?: "none" | "sm" | "base" | "lg";
    /** 可点击：加 hover 态与指针。用于整块跳转的卡片。 */
    interactive?: boolean;
  }>(),
  { padding: "base", interactive: false },
);
</script>

<template>
  <div class="ui-card" :class="[`pad-${padding}`, { interactive }]">
    <slot />
  </div>
</template>

<style scoped>
.ui-card {
  background: var(--aa-surface);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius);
}
.pad-none {
  padding: 0;
}
.pad-sm {
  padding: var(--sp-3);
}
.pad-base {
  padding: var(--sp-4);
}
.pad-lg {
  padding: var(--sp-5);
}
.interactive {
  cursor: pointer;
  transition: border-color var(--motion-fast), background var(--motion-fast);
}
.interactive:hover {
  border-color: var(--aa-primary);
  background: var(--aa-surface-2);
}
</style>
