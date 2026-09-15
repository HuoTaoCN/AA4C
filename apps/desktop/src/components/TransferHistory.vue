<script setup lang="ts">
// 传输记录（UI_DESIGN_SPEC §3.2 ②）。
//
// F3 之前这是独立的「记录」页。记录就是传输的过去时，为它单开一个导航目标，
// 代价是用户想确认「刚才那个发成功没有」得离开当前页——而那正是他刚做完的事。
// 现在它是传输页下半部分的一个区块。
import { computed } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import { useDeviceStore } from "../stores/devices";
import { useSettingsStore } from "../stores/settings";
import { useTransferStore } from "../stores/transfer";
import { useToastStore } from "../stores/toast";
import { Button, Card, EmptyState, Icon } from "./ui";
import { baseName, dayGroup, humanBytes, statusText, timeText } from "../lib/format";
import type { TransferTask } from "../lib/types";

const emit = defineEmits<{ resend: [] }>();

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
  <section class="history">
    <h2>记录</h2>

    <EmptyState
      v-if="transfer.history.length === 0"
      title="还没有传输记录。"
      hint="发出去或收到的文件都会记在这里。"
    />

    <template v-for="g in GROUPS" :key="g">
      <div v-if="grouped[g].length" class="group">
        <h3>{{ g }}</h3>
        <Card padding="none">
          <div v-for="t in grouped[g]" :key="t.id" class="row">
            <span class="dir">{{ t.direction === "send" ? "⬆" : "⬇" }}</span>
            <div class="info">
              <div class="line1">
                <span class="peer">{{ devices.nameOf(t.peer) }}</span>
                <span class="files">{{ summary(t) }}</span>
              </div>
              <div class="line2">
                {{ humanBytes(t.totalBytes) }} · {{ statusText(t.status) }} ·
                {{ timeText(t.createdAt) }}
                <span v-if="t.status === 'failed' && t.error" class="err">
                  · {{ t.error }}
                </span>
              </div>
            </div>
            <div class="ops">
              <Button
                v-if="t.direction === 'recv' && t.status === 'done'"
                size="sm"
                variant="ghost"
                @click="openFolder"
              >
                <Icon name="folder-open" :size="14" /> 打开所在文件夹
              </Button>
              <Button
                v-if="t.status === 'failed'"
                size="sm"
                variant="ghost"
                @click="emit('resend')"
              >
                <Icon name="retry" :size="14" /> 重新发送
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </template>
  </section>
</template>

<style scoped>
.history {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
h2 {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: var(--fw-bold);
  color: var(--aa-text-dim);
}
h3 {
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
}
.row + .row {
  border-top: 1px solid var(--aa-border);
}
.dir {
  flex: none;
  color: var(--aa-text-dim);
}
.info {
  flex: 1;
  min-width: 0;
}
.line1 {
  display: flex;
  gap: var(--sp-2);
  align-items: baseline;
  min-width: 0;
}
.peer {
  font-size: var(--fs-base);
  font-weight: var(--fw-medium);
  flex: none;
}
.files {
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.line2 {
  margin-top: var(--sp-1);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.err {
  color: var(--aa-danger);
}
.ops {
  flex: none;
  display: flex;
  gap: var(--sp-2);
}
</style>
