<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-dialog";
import DownloadCard from "../components/DownloadCard.vue";
import { useDownloadStore } from "../stores/download";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { errorText, humanSpeed, taskTitle } from "../lib/format";
import type { DownloadOptions } from "../lib/types";

const download = useDownloadStore();
const toast = useToastStore();

onMounted(() => {
  void download.load();
});

/** 一行一个链接，支持一次粘贴/输入多条批量添加（对标 FDM/IDM 的批量添加）。 */
const urlsInput = ref("");
const adding = ref(false);

// —— 高级选项（对标 Motrix 新建任务对话框）：默认折叠，绝大多数下载用不到 ——
const showAdvanced = ref(false);
const advanced = reactive({
  saveDir: "",
  out: "",
  referer: "",
  cookie: "",
});
function resetAdvanced() {
  advanced.saveDir = "";
  advanced.out = "";
  advanced.referer = "";
  advanced.cookie = "";
}
/** 把高级选项收成后端要的形状；全空时返回 undefined（等同没有选项）。 */
function currentOptions(): DownloadOptions | undefined {
  const opts: DownloadOptions = {};
  if (advanced.saveDir.trim()) opts.saveDir = advanced.saveDir.trim();
  if (advanced.out.trim()) opts.out = advanced.out.trim();
  if (advanced.referer.trim()) opts.referer = advanced.referer.trim();
  if (advanced.cookie.trim()) opts.cookie = advanced.cookie.trim();
  return Object.keys(opts).length ? opts : undefined;
}
/** 自定义文件名对"一次加多个链接"没有意义（多个任务不能同名），批量时忽略它。 */
const outIgnoredForBatch = computed(
  () =>
    advanced.out.trim().length > 0 &&
    urlsInput.value.split(/\r?\n/).filter((s) => s.trim()).length > 1,
);

async function pickSaveDir() {
  const picked = await open({ directory: true, multiple: false });
  if (typeof picked === "string") advanced.saveDir = picked;
}

/** 选一个本地 .torrent 文件添加 BT 任务（DOWNLOAD_DESIGN 里长期挂着的「仍待实现」）。 */
async function pickTorrentFile() {
  const picked = await open({
    multiple: false,
    filters: [{ name: "种子文件", extensions: ["torrent"] }],
  });
  if (typeof picked !== "string") return;
  adding.value = true;
  try {
    await download.addTorrentFile(picked, currentOptions());
    toast.push("success", "已添加种子任务");
    resetAdvanced();
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  } finally {
    adding.value = false;
  }
}

/** 识别一条文本是否"看起来像"能下载的链接（HTTP/HTTPS/FTP 直链或 magnet）——
 *  剪贴板识别、批量添加分行后都用这条判断过滤掉非链接内容。 */
function looksLikeDownloadLink(s: string): boolean {
  return /^(https?|ftp):\/\/\S+$/i.test(s) || /^magnet:\?xt=urn:btih:/i.test(s);
}

async function add() {
  const lines = urlsInput.value
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  if (!lines.length) return;

  adding.value = true;
  let added = 0;
  let skippedDuplicate = 0;
  let failed = 0;
  try {
    // 批量时丢掉自定义文件名（多个任务不可能同名），其余选项照常应用。
    const base = currentOptions();
    const options =
      base && lines.length > 1 ? { ...base, out: undefined } : base;
    // 批内自身去重 + 跟当前列表去重（同一次粘贴里贴了两遍同一个链接，或者
    // 链接已经在下载列表里——都不重复添加，只提示，不静默失败）。
    const seen = new Set<string>();
    for (const line of lines) {
      if (seen.has(line) || download.findByUrl(line)) {
        skippedDuplicate++;
        continue;
      }
      seen.add(line);
      try {
        await download.add(line, options);
        added++;
      } catch (e) {
        failed++;
        toast.push("error", errorText(asCommandError(e).code));
      }
    }
    urlsInput.value = "";
    if (added > 0) resetAdvanced();
    // 只有一条链接且成功时保持原有"静默清空"体验，不额外弹 toast；批量场景
    // 结果不是一眼能看出来的，需要一条汇总反馈。
    if (lines.length > 1) {
      const parts = [`已添加 ${added} 个下载`];
      if (skippedDuplicate) parts.push(`跳过 ${skippedDuplicate} 个重复链接`);
      if (failed) parts.push(`${failed} 个添加失败`);
      toast.push(failed ? "error" : "success", parts.join("，"));
    } else if (skippedDuplicate) {
      toast.push("info", "这个链接已经在下载列表里了");
    }
  } finally {
    adding.value = false;
  }
}

/** 批量操作（D3）：单个任务失败不影响其余任务，这里只报告实际生效的数量。 */
async function pauseAll() {
  try {
    const n = await download.pauseAll();
    toast.push("info", `已暂停 ${n} 个任务`);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function resumeAll() {
  try {
    const n = await download.resumeAll();
    toast.push("info", `已继续 ${n} 个任务`);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
async function clearCompleted() {
  try {
    const n = await download.clearCompleted();
    toast.push("success", `已清除 ${n} 条记录`);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}

// —— 筛选 + 搜索（列表变长后一眼找到想要的任务，对标 FDM/Motrix 的状态分栏）——
const FILTER_TABS = [
  { key: "all", label: "全部" },
  { key: "active", label: "进行中" },
  { key: "complete", label: "已完成" },
  { key: "error", label: "失败" },
] as const;
type FilterKey = (typeof FILTER_TABS)[number]["key"];
const filter = ref<FilterKey>("all");
const search = ref("");

const filteredList = computed(() => {
  let list = download.list;
  if (filter.value === "active") {
    list = list.filter((t) => t.status === "active" || t.status === "waiting" || t.status === "paused");
  } else if (filter.value === "complete") {
    list = list.filter((t) => t.status === "complete");
  } else if (filter.value === "error") {
    list = list.filter((t) => t.status === "error");
  }
  const q = search.value.trim().toLowerCase();
  if (q) {
    list = list.filter((t) => taskTitle(t.url, t.id).toLowerCase().includes(q));
  }
  return list;
});

// —— 剪贴板自动识别（对标 Motrix/IDM"检测到链接"提示）——
// 只提示、不自动开始下载，避免复制了任意一个链接就被打扰；轮询而不是只监听
// 窗口聚焦——应用在前台时用户也可能复制链接，同步文件监听场景下"轮询兜底"
// 的既有先例（README/DOWNLOAD_DESIGN.md 多处提到的"事件为主、轮询兜底"风格
// 这里反过来是"轮询为主"，因为剪贴板变化没有可订阅的系统事件）。
const CLIPBOARD_POLL_MS = 2000;
const detectedUrl = ref<string | null>(null);
let lastClipboard = "";
let dismissed = new Set<string>();
let clipboardTimer: ReturnType<typeof setInterval> | undefined;

async function pollClipboard() {
  let text: string;
  try {
    text = (await readText()) ?? "";
  } catch {
    return; // 剪贴板不可读（权限/平台限制）：静默跳过，不打扰用户
  }
  const trimmed = text.trim();
  if (trimmed === lastClipboard) return;
  lastClipboard = trimmed;
  if (
    trimmed &&
    looksLikeDownloadLink(trimmed) &&
    !dismissed.has(trimmed) &&
    !download.findByUrl(trimmed)
  ) {
    detectedUrl.value = trimmed;
  }
}

async function acceptDetected() {
  if (!detectedUrl.value) return;
  const url = detectedUrl.value;
  detectedUrl.value = null;
  try {
    await download.add(url);
  } catch (e) {
    toast.push("error", errorText(asCommandError(e).code));
  }
}
function dismissDetected() {
  if (detectedUrl.value) dismissed.add(detectedUrl.value);
  detectedUrl.value = null;
}

onMounted(() => {
  clipboardTimer = setInterval(() => void pollClipboard(), CLIPBOARD_POLL_MS);
});
onUnmounted(() => {
  if (clipboardTimer) clearInterval(clipboardTimer);
});
</script>

<template>
  <div class="download">
    <h2>下载</h2>
    <p class="intro muted">
      粘贴 HTTP / HTTPS / FTP 直链或 magnet 磁力链接即可下载，一行一个可批量添加，
      完成后自然可以走同步/分享继续流动。
    </p>

    <div v-if="detectedUrl" class="card detect">
      <span class="detect-text">
        检测到剪贴板里有个下载链接：<span class="detect-url">{{ detectedUrl }}</span>
      </span>
      <span class="detect-actions">
        <button class="btn btn-primary small" @click="acceptDetected">添加下载</button>
        <button class="btn btn-ghost small" @click="dismissDetected">忽略</button>
      </span>
    </div>

    <div class="card form">
      <div class="row">
        <textarea
          v-model="urlsInput"
          rows="1"
          placeholder="https://… 或 magnet:?xt=…（一行一个，可批量添加）"
          @keydown.enter.exact.prevent="add"
        ></textarea>
        <button class="btn btn-primary" :disabled="adding || !urlsInput.trim()" @click="add">
          {{ adding ? "添加中…" : "开始下载" }}
        </button>
      </div>
      <div class="form-actions">
        <button class="link-btn" type="button" @click="showAdvanced = !showAdvanced">
          {{ showAdvanced ? "▾" : "▸" }} 高级选项
        </button>
        <button class="link-btn" type="button" :disabled="adding" @click="pickTorrentFile">
          📄 选择种子文件…
        </button>
      </div>

      <div v-show="showAdvanced" class="advanced">
        <div class="afield">
          <label>保存到</label>
          <div class="dir">
            <span class="path">{{ advanced.saveDir || "使用默认下载目录" }}</span>
            <button class="btn btn-ghost small" @click="pickSaveDir">选择…</button>
            <button
              v-if="advanced.saveDir"
              class="btn btn-ghost small"
              @click="advanced.saveDir = ''"
            >
              清除
            </button>
          </div>
        </div>
        <div class="afield">
          <label>另存为文件名</label>
          <input v-model="advanced.out" type="text" placeholder="留空用服务器给的名字" />
          <p v-if="outIgnoredForBatch" class="ahint warn">
            ⚠️ 一次添加多个链接时这一项会被忽略（多个任务不能同名）。
          </p>
        </div>
        <div class="afield">
          <label>来源页地址（Referer）</label>
          <input v-model="advanced.referer" type="text" placeholder="有防盗链的站点需要填" />
        </div>
        <div class="afield">
          <label>Cookie</label>
          <input v-model="advanced.cookie" type="text" placeholder="需要登录才能下载时填" />
        </div>
        <p class="ahint muted">
          这些选项只作用于这次添加的任务；种子任务只有「保存到」有效。
        </p>
      </div>
    </div>

    <div v-if="download.list.length" class="stats muted">
      <span v-if="download.activeCount">
        {{ download.activeCount }} 个进行中 · 总速度 {{ humanSpeed(download.totalSpeedBps) }}
      </span>
      <span v-else>共 {{ download.list.length }} 个任务</span>
    </div>

    <div
      v-if="download.hasActiveOrWaiting || download.hasPaused || download.hasCompleted"
      class="batch"
    >
      <button v-if="download.hasActiveOrWaiting" class="btn btn-ghost small" @click="pauseAll">
        全部暂停
      </button>
      <button v-if="download.hasPaused" class="btn btn-ghost small" @click="resumeAll">
        全部继续
      </button>
      <button v-if="download.hasCompleted" class="btn btn-ghost small" @click="clearCompleted">
        清除已完成
      </button>
    </div>

    <div v-if="download.list.length" class="toolbar">
      <div class="tabs">
        <button
          v-for="tab in FILTER_TABS"
          :key="tab.key"
          class="tab"
          :class="{ on: filter === tab.key }"
          @click="filter = tab.key"
        >
          {{ tab.label }}
        </button>
      </div>
      <input v-model="search" type="text" class="search" placeholder="搜索任务名…" />
    </div>

    <div v-if="filteredList.length" class="card list">
      <div v-for="t in filteredList" :key="t.id" class="drow">
        <DownloadCard :task="t" />
      </div>
    </div>
    <div v-else-if="download.list.length" class="empty card muted">没有匹配的任务。</div>
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
  margin-bottom: 10px;
}
.row {
  display: flex;
  gap: 10px;
  align-items: flex-end;
}
.row textarea {
  flex: 1;
  min-height: 38px;
  max-height: 140px;
  padding: 9px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.9rem;
  font-family: inherit;
  resize: vertical;
  line-height: 1.4;
}
.form-actions {
  display: flex;
  gap: 16px;
  margin-top: 10px;
}
.link-btn {
  font-size: 0.8rem;
  color: var(--aa-text-dim);
  padding: 2px 0;
}
.link-btn:hover:not(:disabled) {
  color: var(--aa-primary);
}
.link-btn:disabled {
  opacity: 0.5;
}
.advanced {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid var(--aa-border);
}
.afield {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.afield label {
  font-size: 0.8rem;
  font-weight: 600;
}
.afield input {
  padding: 7px 10px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.85rem;
}
.afield .dir {
  display: flex;
  align-items: center;
  gap: 8px;
}
.afield .path {
  flex: 1;
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  word-break: break-all;
}
.afield .small {
  padding: 4px 10px;
  min-height: 28px;
  font-size: 0.78rem;
  flex-shrink: 0;
}
.ahint {
  font-size: 0.76rem;
  line-height: 1.55;
  margin: 0;
}
.ahint.warn {
  color: #9a6a00;
}
@media (prefers-color-scheme: dark) {
  .ahint.warn {
    color: #ffce80;
  }
}
.detect {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
  margin-bottom: 10px;
  background: var(--aa-surface-2);
  border-left: 3px solid var(--aa-primary);
}
.detect-text {
  font-size: 0.85rem;
  flex: 1;
  min-width: 0;
}
.detect-url {
  font-weight: 600;
  word-break: break-all;
}
.detect-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.stats {
  font-size: 0.78rem;
  margin: 0 2px 10px;
}
.batch {
  display: flex;
  gap: 8px;
  margin-bottom: 10px;
}
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 10px;
}
.tabs {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  background: var(--aa-surface-2);
  border-radius: var(--aa-radius-sm);
  flex-shrink: 0;
}
.tab {
  padding: 5px 12px;
  font-size: 0.8rem;
  color: var(--aa-text-dim);
  border-radius: calc(var(--aa-radius-sm) - 3px);
  white-space: nowrap;
}
.tab.on {
  background: var(--aa-surface);
  color: var(--aa-text);
  font-weight: 600;
}
.search {
  flex: 1;
  min-width: 0;
  padding: 6px 10px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.82rem;
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
