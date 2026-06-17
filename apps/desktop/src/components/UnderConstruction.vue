<script setup lang="ts">
defineProps<{
  name: string;
  icon: string;
  desc: string;
  version?: string;
}>();
</script>

<template>
  <div class="uc">
    <div class="badge">🚧 建设中</div>
    <div class="icon">{{ icon }}</div>
    <h2>{{ name }}</h2>
    <p class="desc">{{ desc }}</p>
    <p v-if="version" class="ver">计划于 {{ version }} 推出</p>
    <router-link to="/" class="btn btn-ghost">返回首页</router-link>
  </div>
</template>

<style scoped>
.uc {
  max-width: 460px;
  margin: 8vh auto 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 12px;
}
.badge {
  font-size: 0.78rem;
  font-weight: 600;
  color: #9a6a00;
  background: #ffedcc;
  padding: 4px 12px;
  border-radius: 999px;
}
@media (prefers-color-scheme: dark) {
  .badge {
    color: #ffce80;
    background: #4a3a16;
  }
}
.icon {
  font-size: 3.2rem;
  margin-top: 6px;
}
h2 {
  margin: 0;
  font-size: 1.3rem;
}
.desc {
  margin: 0;
  color: var(--aa-text-dim);
  line-height: 1.6;
}
.ver {
  margin: 0;
  font-size: 0.85rem;
  color: var(--aa-text-dim);
}
.uc .btn {
  margin-top: 10px;
}
</style>
