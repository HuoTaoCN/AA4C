<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { openPath } from "@tauri-apps/plugin-opener";
import { useDeviceStore } from "../stores/devices";
import { useSettingsStore } from "../stores/settings";
import { useTransferStore } from "../stores/transfer";
import { useToastStore } from "../stores/toast";
import {
  baseName,
  dayGroup,
  humanBytes,
  statusText,
  timeText,
} from "../lib/format";
import type { TransferTask } from "../lib/types";

const router = useRouter();
const devices = useDeviceStore();
const settings = useSettingsStore();
const transfer = useTransferStore();
const toast = useToastStore();

const GROUPS = ["今天", "昨天", "更早"] as const;

const grouped = computed(() => {
  const out: Record<string, TransferTask[]> = { 今天: [], 昨天: [], 更早: [] };
  for (const t of transfer.history) out[dayGroup(t.createdAt)].push(t);
  return out;
});

function summary(t: TransferTask): string {
  if (t.files.length === 0) return "文件";
  const first = baseName(t.files[0].relPath);
  return t.files.length === 1 ? first : `${first} 等 ${t.files.length} 个文件`;
}

async function openFolder() {
  const dir = settings.settings?.saveDir;
  if (!dir) return;
  try {
    await openPath(dir);
  } catch {
    toast.push("error", "打不开文件夹，请到设置查看保存位置");
  }
}
</script>

<template>
  <div class="records">
    <h2>记录</h2>
    <div v-if="transfer.history.length === 0" class="empty card muted">
      还没有传输记录。
    </div>
    <template v-for="g in GROUPS" :key="g">
      <section v-if="grouped[g].length" class="group">
        <h3>{{ g }}</h3>
        <ul class="card">
          <li v-for="t in grouped[g]" :key="t.id">
            <span class="dir">{{ t.direction === "send" ? "⬆" : "⬇" }}</span>
            <div class="info">
              <div class="line1">
                <span class="peer">{{ devices.nameOf(t.peer) }}</span>
                <span class="files muted">{{ summary(t) }}</span>
              </div>
              <div class="line2 muted">
                {{ humanBytes(t.totalBytes) }} · {{ statusText(t.status) }} ·
                {{ timeText(t.createdAt) }}
                <span v-if="t.status === 'failed' && t.error" class="err">
                  · {{ t.error }}
                </span>
              </div>
            </div>
            <div class="ops">
              <button
                v-if="t.direction === 'recv' && t.status === 'done'"
                class="link"
                @click="openFolder"
              >
                打开所在文件夹
              </button>
              <button
                v-if="t.status === 'failed'"
                class="link"
                @click="router.push('/send')"
              >
                重新发送
              </button>
            </div>
          </li>
        </ul>
      </section>
    </template>
  </div>
</template>

<style scoped>
.records {
  max-width: 820px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 16px;
}
h3 {
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  margin: 18px 0 8px;
}
.empty {
  padding: 30px;
  text-align: center;
}
ul {
  list-style: none;
  margin: 0;
  padding: 4px 0;
}
li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
}
li + li {
  border-top: 1px solid var(--aa-border);
}
.dir {
  font-size: 1.1rem;
}
.info {
  flex: 1;
  min-width: 0;
}
.line1 {
  display: flex;
  gap: 10px;
  align-items: baseline;
  min-width: 0;
}
.peer {
  font-weight: 600;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.files {
  font-size: 0.85rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.line2 {
  font-size: 0.78rem;
  margin-top: 2px;
}
.err {
  color: var(--aa-danger);
}
.link {
  color: var(--aa-primary);
  font-weight: 600;
  font-size: 0.82rem;
  white-space: nowrap;
}
</style>
