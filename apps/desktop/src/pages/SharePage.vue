<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useShareStore } from "../stores/share";
import { useSyncStore } from "../stores/sync";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { humanBytes, timeText } from "../lib/format";
import {
  Button,
  Card,
  EmptyState,
  Field,
  Icon,
  ListRow,
  Toolbar,
} from "../components/ui";

const share = useShareStore();
const sync = useSyncStore();
const toast = useToastStore();

onMounted(() => {
  void share.load();
  void sync.load();
});

// —— 生成分享 ——
const localFiles = computed(() => sync.files.filter((f) => f.status === "local"));
const EXPIRY_OPTIONS = [
  { key: "1h", label: "1 小时后过期", ms: 60 * 60 * 1000 },
  { key: "1d", label: "1 天后过期", ms: 24 * 60 * 60 * 1000 },
  { key: "7d", label: "7 天后过期", ms: 7 * 24 * 60 * 60 * 1000 },
  { key: "forever", label: "长期有效", ms: null as number | null },
];
const form = reactive({ relPath: "", expiry: "7d" });
const creating = ref(false);

async function createShare() {
  if (!form.relPath) {
    toast.push("error", "先选一个要分享的文件");
    return;
  }
  const opt = EXPIRY_OPTIONS.find((o) => o.key === form.expiry);
  const expiresAt = opt?.ms == null ? null : Date.now() + opt.ms;
  creating.value = true;
  try {
    const s = await share.create(form.relPath, expiresAt);
    await copyLink(s.link);
    toast.push("success", "分享链接已生成并复制到剪贴板");
    form.relPath = "";
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  } finally {
    creating.value = false;
  }
}

async function copyLink(link: string) {
  try {
    await navigator.clipboard.writeText(link);
  } catch {
    // 剪贴板权限被拒也不阻塞——链接本来就在列表里可以再复制
  }
}

async function revoke(id: string) {
  try {
    await share.revoke(id);
    toast.push("info", "已吊销该分享");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

// —— 打开分享链接 ——
const openLink = ref("");
const opening = ref(false);

async function openShare() {
  const link = openLink.value.trim();
  if (!link) return;
  opening.value = true;
  try {
    await share.open(link);
    toast.push("success", "已开始接收，进度见底部任务栏");
    openLink.value = "";
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  } finally {
    opening.value = false;
  }
}

function expiryText(s: { expiresAt: number | null; status: string }): string {
  if (s.status === "revoked") return "已吊销";
  if (s.expiresAt == null) return "长期有效";
  return s.expiresAt <= Date.now() ? "已过期" : `${timeText(s.expiresAt)} 过期`;
}
</script>

<template>
  <div class="share">
    <Toolbar
      title="分享"
      subtitle="生成一条链接，给还没配对的人——对方粘贴链接即可取回"
    />

    <Card>
      <Field
        label="选一个要分享的文件"
        :hint="
          localFiles.length
            ? undefined
            : '还没有本地文件可分享——先在「同步」页添加一个同步文件夹。'
        "
      >
        <select v-model="form.relPath">
          <option value="" disabled>请选择…</option>
          <option v-for="f in localFiles" :key="f.basePath" :value="f.basePath">
            {{ f.relPath }}（{{ humanBytes(f.size) }}）
          </option>
        </select>
      </Field>

      <Field label="有效期">
        <select v-model="form.expiry">
          <option v-for="o in EXPIRY_OPTIONS" :key="o.key" :value="o.key">
            {{ o.label }}
          </option>
        </select>
      </Field>

      <div class="actions">
        <Button
          variant="primary"
          :disabled="creating || !form.relPath"
          @click="createShare"
        >
          {{ creating ? "生成中…" : "生成分享链接" }}
        </Button>
      </div>
    </Card>

    <Card>
      <Field label="打开一个分享链接">
        <div class="row">
          <input v-model="openLink" type="text" placeholder="aa4c://share/…" />
          <Button
            variant="primary"
            :disabled="opening || !openLink.trim()"
            @click="openShare"
          >
            {{ opening ? "打开中…" : "打开" }}
          </Button>
        </div>
      </Field>
    </Card>

    <section>
      <h2>我的分享</h2>
      <Card v-if="share.shares.length" padding="none">
        <ListRow
          v-for="s in share.shares"
          :key="s.id"
          :title="s.relPath"
          :status="expiryText(s)"
          :tone="s.status === 'revoked' ? 'muted' : 'ok'"
        >
          <template #detail>
            <div class="slink">
              <code>{{ s.link }}</code>
              <Button size="sm" variant="ghost" @click="copyLink(s.link)">
                <Icon name="copy" :size="14" /> 复制
              </Button>
            </div>
          </template>
          <template #actions>
            <Button
              size="sm"
              variant="danger"
              :disabled="s.status === 'revoked'"
              @click="revoke(s.id)"
            >
              吊销
            </Button>
          </template>
        </ListRow>
      </Card>
      <EmptyState
        v-else
        title="还没有生成过分享链接。"
        hint="选一个本地文件生成链接，发给对方即可——不需要你在线盯着。"
      />
    </section>
  </div>
</template>

<style scoped>
.share {
  max-width: 720px;
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
}
section {
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
.actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--sp-3);
}
.row {
  display: flex;
  gap: var(--sp-2);
}
.row input {
  flex: 1;
  min-width: 0;
}
.slink {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin-top: var(--sp-2);
  min-width: 0;
}
.slink code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
select,
input[type="text"] {
  width: 100%;
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font: inherit;
  font-size: var(--fs-base);
}
</style>
