<script setup lang="ts">
// 全应用唯一的按钮实现（UI_DESIGN_SPEC §5.2）。
withDefaults(
  defineProps<{
    variant?: "primary" | "ghost" | "danger" | "plain";
    size?: "sm" | "base";
    disabled?: boolean;
  }>(),
  { variant: "plain", size: "base", disabled: false },
);
</script>

<template>
  <button
    class="ui-btn"
    :class="[`v-${variant}`, `s-${size}`]"
    :disabled="disabled"
  >
    <slot />
  </button>
</template>

<style scoped>
.ui-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--sp-1);
  border-radius: var(--aa-radius-sm);
  font-size: var(--fs-base);
  font-weight: var(--fw-bold);
  background: var(--aa-surface-2);
  color: var(--aa-text);
  transition: filter var(--motion-fast), background var(--motion-fast);
}
.s-base {
  padding: var(--sp-2) var(--sp-4);
  /* 触控目标最小 44px 是移动端的硬要求（§11）；桌面上 38px 够用，
     移动布局里由下面的媒体查询顶上去。 */
  min-height: 38px;
}
.s-sm {
  padding: var(--sp-1) var(--sp-3);
  font-size: var(--fs-sm);
  min-height: 30px;
}
.ui-btn:hover:not(:disabled) {
  filter: brightness(0.97);
}
.ui-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.v-primary {
  background: var(--aa-primary);
  color: #fff;
}
.v-ghost {
  background: transparent;
  border: 1px solid var(--aa-border);
}
.v-danger {
  background: transparent;
  color: var(--aa-danger);
  border: 1px solid var(--aa-border);
}

@media (max-width: 699px) {
  .s-base {
    min-height: 44px;
  }
  .s-sm {
    min-height: 36px;
  }
}
</style>
