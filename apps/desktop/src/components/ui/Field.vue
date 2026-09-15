<script setup lang="ts">
// 设置项一行：标签 + 控件 + 提示（UI_DESIGN_SPEC §5.2）。
//
// `row` 给开关用（标签与控件同一行），默认给输入框用（控件换行占满）。
withDefaults(
  defineProps<{
    label: string;
    hint?: string;
    /** 提示的语气。`warn` 用于「这会在你的路由器上开一个端口」这类要让人看见的话。 */
    tone?: "muted" | "warn";
    row?: boolean;
  }>(),
  { tone: "muted", row: false },
);
</script>

<template>
  <div class="ui-field" :class="{ row }">
    <label>{{ label }}</label>
    <div class="control"><slot /></div>
    <p v-if="hint" class="hint" :class="`t-${tone}`">{{ hint }}</p>
  </div>
</template>

<style scoped>
.ui-field {
  display: grid;
  gap: var(--sp-2);
  padding: var(--sp-3) 0;
}
.ui-field + .ui-field {
  border-top: 1px solid var(--aa-border);
}
.ui-field.row {
  grid-template-columns: 1fr auto;
  align-items: center;
}
.ui-field.row .hint {
  grid-column: 1 / -1;
}
label {
  font-size: var(--fs-base);
}
.hint {
  margin: 0;
  font-size: var(--fs-sm);
  line-height: 1.5;
}
.t-muted {
  color: var(--aa-text-dim);
}
.t-warn {
  color: var(--aa-warn);
}
</style>
