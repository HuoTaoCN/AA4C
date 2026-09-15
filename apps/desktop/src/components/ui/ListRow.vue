<script setup lang="ts">
// 一行：图标 + 主副文案 + 右侧状态 + 操作区（UI_DESIGN_SPEC §5.2）。
//
// 设备行、分享行、模型行、来源行……应用里到处是这个形状，F3 之前每处各写一遍。
withDefaults(
  defineProps<{
    /** 主文案。 */
    title: string;
    /** 副文案（信任级别、时间、路径…）。 */
    subtitle?: string;
    /** 右侧状态文案（「已连接 · 局域网直连」）。 */
    status?: string;
    /** 状态语气，决定右侧文案的颜色。 */
    tone?: "ok" | "warn" | "danger" | "muted";
    interactive?: boolean;
  }>(),
  { tone: "muted", interactive: false },
);
</script>

<template>
  <div class="ui-row" :class="{ interactive }">
    <span v-if="$slots.icon" class="icon"><slot name="icon" /></span>
    <div class="text">
      <div class="title">{{ title }}</div>
      <div v-if="subtitle" class="sub">{{ subtitle }}</div>
      <!-- 展开区：连不上时的原因与下一步放这里（§3.1） -->
      <slot name="detail" />
    </div>
    <div class="right">
      <span v-if="status" class="status" :class="`t-${tone}`">{{ status }}</span>
      <div v-if="$slots.actions" class="actions"><slot name="actions" /></div>
    </div>
  </div>
</template>

<style scoped>
.ui-row {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
}
.ui-row + .ui-row {
  border-top: 1px solid var(--aa-border);
}
.interactive {
  cursor: pointer;
  transition: background var(--motion-fast);
}
.interactive:hover {
  background: var(--aa-surface-2);
}
.icon {
  font-size: var(--fs-xl);
  line-height: 1.2;
  flex: none;
}
.text {
  flex: 1;
  min-width: 0;
}
.title {
  font-size: var(--fs-base);
  font-weight: var(--fw-medium);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub {
  margin-top: var(--sp-1);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.right {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--sp-2);
  flex: none;
}
.status {
  font-size: var(--fs-sm);
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
.actions {
  display: flex;
  gap: var(--sp-2);
}
</style>
