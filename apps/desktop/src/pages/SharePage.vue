<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useShareStore } from "../stores/share";
import { useSyncStore } from "../stores/sync";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { humanBytes, timeText } from "../lib/format";

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
    <h2>分享</h2>
    <p class="intro muted">
      把已经同步到本机的文件生成一个链接，发给已配对的朋友——对方粘贴链接即可取回，不需要
      你在线盯着（局域网内不依赖服务器；跨网络可达随「远程连接」设置就绪自然生效）。
    </p>

    <div class="card form">
      <div class="field">
        <label>选一个要分享的文件</label>
        <select v-model="form.relPath">
          <option value="" disabled>请选择…</option>
          <option v-for="f in localFiles" :key="f.basePath" :value="f.basePath">
            {{ f.relPath }}（{{ humanBytes(f.size) }}）
          </option>
        </select>
        <p v-if="!localFiles.length" class="hint muted">
          还没有本地文件可分享——先在「同步」页添加一个同步文件夹。
        </p>
      </div>
      <div class="field">
        <label>有效期</label>
        <select v-model="form.expiry">
          <option v-for="o in EXPIRY_OPTIONS" :key="o.key" :value="o.key">{{ o.label }}</option>
        </select>
      </div>
      <div class="actions">
        <button class="btn btn-primary" :disabled="creating || !form.relPath" @click="createShare">
          {{ creating ? "生成中…" : "生成分享链接" }}
        </button>
      </div>
    </div>

    <div class="card form">
      <div class="field">
        <label>打开一个分享链接</label>
        <div class="row">
          <input v-model="openLink" type="text" placeholder="aa4c://share/…" />
          <button class="btn btn-primary small" :disabled="opening || !openLink.trim()" @click="openShare">
            {{ opening ? "打开中…" : "打开" }}
          </button>
        </div>
      </div>
    </div>

    <h3>我的分享</h3>
    <div v-if="share.shares.length" class="card list">
      <div v-for="s in share.shares" :key="s.id" class="srow">
        <div class="sinfo">
          <div class="sname">
            <span class="nm">{{ s.relPath }}</span>
            <span class="tag" :class="{ off: s.status === 'revoked' }">{{ expiryText(s) }}</span>
          </div>
          <div class="slink">
            <span class="link">{{ s.link }}</span>
            <button class="btn btn-ghost small" @click="copyLink(s.link)">复制</button>
          </div>
        </div>
        <button
          class="btn btn-danger small"
          :disabled="s.status === 'revoked'"
          @click="revoke(s.id)"
        >
          吊销
        </button>
      </div>
    </div>
    <div v-else class="empty card muted">还没有生成过分享链接。</div>
  </div>
</template>

<style scoped>
.share {
  max-width: 640px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 8px;
}
h3 {
  font-size: 0.85rem;
  margin: 26px 0 10px;
}
.intro {
  font-size: 0.85rem;
  line-height: 1.6;
  margin: 0 0 16px;
}
.form {
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  margin-bottom: 14px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
label {
  font-size: 0.88rem;
  font-weight: 600;
}
select,
input[type="text"] {
  padding: 9px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.9rem;
}
.row {
  display: flex;
  gap: 10px;
}
.row input {
  flex: 1;
}
.actions {
  display: flex;
  justify-content: flex-end;
}
.small {
  padding: 5px 12px;
  min-height: 32px;
  font-size: 0.8rem;
}
.hint {
  font-size: 0.78rem;
}
.list {
  padding: 4px 0;
}
.srow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
}
.srow + .srow {
  border-top: 1px solid var(--aa-border);
}
.sinfo {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.sname {
  display: flex;
  align-items: center;
  gap: 8px;
}
.nm {
  font-weight: 600;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tag {
  flex-shrink: 0;
  font-size: 0.68rem;
  font-weight: 600;
  color: #9a6a00;
  background: #ffedcc;
  padding: 1px 8px;
  border-radius: 999px;
}
.tag.off {
  color: var(--aa-text-dim);
  background: var(--aa-surface-2);
}
@media (prefers-color-scheme: dark) {
  .tag {
    color: #ffce80;
    background: #4a3a16;
  }
}
.slink {
  display: flex;
  align-items: center;
  gap: 8px;
}
.link {
  flex: 1;
  min-width: 0;
  font-size: 0.78rem;
  color: var(--aa-text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: monospace;
}
.empty {
  padding: 24px;
  text-align: center;
}
</style>
