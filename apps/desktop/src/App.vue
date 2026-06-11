<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const version = ref("");

onMounted(async () => {
  version.value = await invoke<string>("aa4c_version");
});
</script>

<template>
  <main class="welcome">
    <h1 class="brand">AA4C</h1>
    <p class="slogan">让所有设备成为一个空间</p>
    <p class="version" v-if="version">v{{ version }}</p>
  </main>
</template>

<style scoped>
.welcome {
  height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.75rem;
  font-family: -apple-system, "PingFang SC", "Microsoft YaHei", system-ui,
    sans-serif;
}

.brand {
  font-size: 4rem;
  font-weight: 800;
  letter-spacing: 0.05em;
  color: var(--aa-primary, #2f6bff);
  margin: 0;
}

.slogan {
  font-size: 1.25rem;
  color: #555;
  margin: 0;
}

.version {
  font-size: 0.875rem;
  color: #999;
  margin: 0;
}

@media (prefers-color-scheme: dark) {
  .slogan {
    color: #bbb;
  }
}
</style>
