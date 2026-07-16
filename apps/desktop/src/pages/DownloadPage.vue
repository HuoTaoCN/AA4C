<script setup lang="ts">
import { onMounted, ref } from "vue";
import DownloadCard from "../components/DownloadCard.vue";
import { useDownloadStore } from "../stores/download";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { errorText } from "../lib/format";

const download = useDownloadStore();
const toast = useToastStore();

onMounted(() => {
  void download.load();
});

const url = ref("");
const adding = ref(false);

async function add() {
  const value = url.value.trim();
  if (!value) return;
  adding.value = true;
  try {
    await download.add(value);
    url.value = "";
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  } finally {
    adding.value = false;
  }
}
</script>

<template>
  <div class="download">
    <h2>下载</h2>
    <p class="intro muted">
      粘贴一条 HTTP / HTTPS / FTP 直链，或者一条 magnet 磁力链接，即可下载，完成后
      自然可以走同步/分享继续流动。
    </p>

    <div class="card form">
      <div class="row">
        <input
          v-model="url"
          type="text"
          placeholder="https://… 或 magnet:?xt=…"
          @keyup.enter="add"
        />
        <button class="btn btn-primary" :disabled="adding || !url.trim()" @click="add">
          {{ adding ? "添加中…" : "开始下载" }}
        </button>
      </div>
    </div>

    <div v-if="download.list.length" class="card list">
      <div v-for="t in download.list" :key="t.id" class="drow">
        <DownloadCard :task="t" />
      </div>
    </div>
    <div v-else class="empty card muted">还没有下载任务。</div>
  </div>
</template>

<style scoped>
.download {
  max-width: 640px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 8px;
}
.intro {
  font-size: 0.85rem;
  line-height: 1.6;
  margin: 0 0 16px;
}
.form {
  padding: 18px 20px;
  margin-bottom: 14px;
}
.row {
  display: flex;
  gap: 10px;
}
.row input {
  flex: 1;
  padding: 9px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.9rem;
}
.list {
  padding: 4px 0;
}
.drow {
  padding: 12px 16px;
}
.drow + .drow {
  border-top: 1px solid var(--aa-border);
}
.empty {
  padding: 24px;
  text-align: center;
}
</style>
