<script setup lang="ts">
// 弹层（UI_DESIGN_SPEC §5.2）：桌面居中，移动端吸底。
//
// 替代各页自己写的 dialog——此前配对、接收两个弹窗各写一套遮罩与动画，
// z-index 还是手写的魔数（弹层被全局任务条盖住就是这么来的）。
defineProps<{ open: boolean; title?: string }>();
const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="ui-sheet-mask" @click.self="emit('close')">
      <div class="sheet" role="dialog" aria-modal="true">
        <h2 v-if="title">{{ title }}</h2>
        <slot />
        <div v-if="$slots.actions" class="actions"><slot name="actions" /></div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ui-sheet-mask {
  position: fixed;
  inset: 0;
  z-index: var(--z-sheet);
  background: rgb(0 0 0 / 35%);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--sp-4);
}
.sheet {
  background: var(--aa-surface);
  border-radius: var(--aa-radius-lg);
  padding: var(--sp-5);
  width: min(420px, 100%);
  max-height: 80vh;
  overflow: auto;
}
h2 {
  margin: 0 0 var(--sp-4);
  font-size: var(--fs-lg);
  font-weight: var(--fw-bold);
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--sp-2);
  margin-top: var(--sp-5);
}

/* 移动端吸底：够得着拇指，也是系统弹层的惯常形态。 */
@media (max-width: 699px) {
  .ui-sheet-mask {
    align-items: flex-end;
    padding: 0;
  }
  .sheet {
    width: 100%;
    border-radius: var(--aa-radius-lg) var(--aa-radius-lg) 0 0;
    padding-bottom: calc(var(--sp-5) + env(safe-area-inset-bottom));
  }
}
</style>
