<script setup lang="ts">
import { computed, onMounted, reactive, ref, watchEffect } from "vue";
import TabBar from "../components/TabBar.vue";
import { useDeviceStore } from "../stores/devices";
import { useSettingsStore } from "../stores/settings";
import { useToastStore } from "../stores/toast";
import { api, asCommandError } from "../lib/api";
import { platformIcon } from "../lib/format";
import type { Settings, SyncScope, TrustLevel } from "../lib/types";

const devices = useDeviceStore();
const settings = useSettingsStore();
const toast = useToastStore();

// —— 页内分区（原先 6 个纵向堆叠区块合成一页太长，按用户任务分组；
// 一个 `form` 对应一次完整保存，所以只留一个全局「保存」按钮，不是每区块一个）——
const SETTINGS_TABS = [
  { key: "general", label: "通用" },
  { key: "remote", label: "远程连接" },
  { key: "download", label: "下载" },
  { key: "archive-ai", label: "归档与 AI" },
  { key: "devices", label: "已配对设备" },
];
const activeTab = ref<"general" | "remote" | "download" | "archive-ai" | "devices">("general");

// 本地编辑副本，从 store 同步
const form = reactive<Settings>({
  deviceName: "",
  saveDir: "",
  autoAcceptFromTrusted: false,
  listenPort: 42420,
  serverUrl: null,
  enableRemote: false,
  downloadDir: "",
  downloadSpeedLimitKbps: null,
  downloadConcurrency: null,
  btRatioLimit: null,
  btIdleSeedingLimitMinutes: null,
  archiveRoot: "",
  archiveAutoEnabled: true,
  aiModelsDir: "",
  aiChatModel: null,
  aiEmbeddingModel: null,
  aiIdleTimeoutMinutes: 10,
});
// 服务器地址单独用字符串编辑（空字符串 ⇄ null，避免保存一个全是空格的"已配置"假象）
const serverUrlInput = computed({
  get: () => form.serverUrl ?? "",
  set: (v: string) => {
    form.serverUrl = v.trim() === "" ? null : v.trim();
  },
});
watchEffect(() => {
  if (settings.settings) Object.assign(form, settings.settings);
});

// 限速/并发/分享率/做种超时都是"留空=不限"的数字输入框，用字符串中转，
// 避免 <input type="number"> 空值时 Vue 把 v-model 强转成 0（0 跟"没设"含义
// 完全不同——0 会被后端当成"限速为 0"，不是"不限速"）。
type NullableNumberKey =
  | "downloadSpeedLimitKbps"
  | "downloadConcurrency"
  | "btRatioLimit"
  | "btIdleSeedingLimitMinutes";
function numberInput(key: NullableNumberKey) {
  return computed({
    get: () => (form[key] === null ? "" : String(form[key])),
    set: (v: string) => {
      const trimmed = v.trim();
      form[key] = trimmed === "" ? null : Number(trimmed);
    },
  });
}
const speedLimitInput = numberInput("downloadSpeedLimitKbps");
const concurrencyInput = numberInput("downloadConcurrency");
const ratioLimitInput = numberInput("btRatioLimit");
const idleSeedingLimitInput = numberInput("btIdleSeedingLimitMinutes");

// 下载目录同步范围重叠警示（D3，DOWNLOAD_DESIGN.md §5）：纯前端路径前缀比对，
// 不阻断保存，只是提醒——`list_sync_scopes` 是既有命令（C6 起就有）。
const scopes = ref<SyncScope[]>([]);
onMounted(async () => {
  try {
    scopes.value = await api.listSyncScopes();
  } catch {
    // 拿不到共享范围列表就不显示警示，不影响设置页其余功能。
  }
});
function normalizePath(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/+$/, "");
}
function findOverlappingScope(path: string): SyncScope | null {
  const target = normalizePath(path);
  if (!target) return null;
  return (
    scopes.value.find((s) => {
      const scopePath = normalizePath(s.localPath);
      return target === scopePath || target.startsWith(`${scopePath}/`);
    }) ?? null
  );
}
const downloadDirWarningScope = computed<SyncScope | null>(() =>
  findOverlappingScope(form.downloadDir),
);
// 归档根目录同样可能落在共享范围内（同下载目录一样的既有隔离原则，ARCHIVE_DESIGN §2.5）。
const archiveRootWarningScope = computed<SyncScope | null>(() =>
  findOverlappingScope(form.archiveRoot),
);

const paired = computed(() => devices.devices.filter((d) => d.trusted));

// 信任分级：真实 trust_level（配对默认 friend，可标记为「我的设备」full）。
const tierOf = (d: { trustLevel: TrustLevel | null }): TrustLevel =>
  d.trustLevel ?? "friend";

async function setTier(id: string, t: TrustLevel) {
  try {
    await api.setTrustLevel(id, t);
    await devices.loadDevices();
    toast.push(
      "info",
      t === "full"
        ? "已标记为「我的设备」——同步功能 V0.2 上线后将参与跨设备文件同步"
        : "已设为「朋友」",
    );
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function changeDir() {
  const picked = await settings.pickSaveDir();
  if (picked) form.saveDir = picked;
}

async function changeDownloadDir() {
  const picked = await settings.pickSaveDir();
  if (picked) form.downloadDir = picked;
}

async function changeArchiveRoot() {
  const picked = await settings.pickSaveDir();
  if (picked) form.archiveRoot = picked;
}

async function changeAiModelsDir() {
  const picked = await settings.pickSaveDir();
  if (picked) form.aiModelsDir = picked;
}

async function save() {
  try {
    await settings.save({ ...form });
    await devices.loadSelf();
    toast.push("success", "设置已保存");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function unpair(id: string) {
  try {
    await api.unpairDevice(id);
    await devices.loadDevices();
    toast.push("info", "已解除配对");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="settings">
    <h2>设置</h2>

    <TabBar v-model="activeTab" :tabs="SETTINGS_TABS" />

    <section v-show="activeTab === 'general'">
      <div class="card form">
        <div class="field">
          <label>设备名称</label>
          <input v-model="form.deviceName" type="text" maxlength="40" />
        </div>

        <div class="field">
          <label>接收文件保存到</label>
          <div class="dir">
            <span class="path">{{ form.saveDir || "默认接收目录" }}</span>
            <button class="btn btn-ghost small" @click="changeDir">更改</button>
          </div>
        </div>

        <div class="field row">
          <label>信任设备来的文件自动接收</label>
          <label class="switch">
            <input type="checkbox" v-model="form.autoAcceptFromTrusted" />
            <span class="slider"></span>
          </label>
        </div>

        <p class="lock muted">🔒 所有传输均已加密</p>
      </div>
    </section>

    <section v-show="activeTab === 'remote'">
      <div class="card form">
        <div class="field">
          <label>自建服务器地址</label>
          <input
            v-model="serverUrlInput"
            type="text"
            placeholder="aa4c://your-server:42420#指纹"
          />
          <p class="hint muted">
            填自己搭的 <code>aa4c-server</code> 地址后，不在同一局域网的已配对设备也能互相找到、
            经中继收发文件——没有服务器不影响局域网内的正常使用。
          </p>
        </div>

        <div class="field row">
          <label>开启远程连接</label>
          <label class="switch">
            <input
              type="checkbox"
              v-model="form.enableRemote"
              :disabled="!form.serverUrl"
            />
            <span class="slider"></span>
          </label>
        </div>
        <p v-if="!form.serverUrl" class="hint muted">先填服务器地址才能打开这个开关。</p>
      </div>
    </section>

    <section v-show="activeTab === 'download'">
      <div class="card form">
        <div class="field">
          <label>下载目录</label>
          <div class="dir">
            <span class="path">{{ form.downloadDir || "默认下载目录" }}</span>
            <button class="btn btn-ghost small" @click="changeDownloadDir">更改</button>
          </div>
          <p v-if="downloadDirWarningScope" class="hint warn">
            ⚠️ 这个目录在共享范围「{{
              downloadDirWarningScope.kind === "inbox" ? "收到的" : downloadDirWarningScope.localPath
            }}」内，保存到这里的文件会被同步给完全信任设备。
          </p>
        </div>

        <div class="field">
          <label>下载限速（KB/s）</label>
          <input v-model="speedLimitInput" type="number" min="0" placeholder="不限速" />
        </div>

        <div class="field">
          <label>同时下载数</label>
          <input v-model="concurrencyInput" type="number" min="1" placeholder="使用默认值" />
        </div>

        <div class="field">
          <label>BT 分享率上限</label>
          <input v-model="ratioLimitInput" type="number" min="0" step="0.1" placeholder="不限" />
        </div>

        <div class="field">
          <label>BT 空闲做种超时（分钟）</label>
          <input v-model="idleSeedingLimitInput" type="number" min="1" placeholder="不限" />
          <p class="hint muted">
            指没有上传活动多久后自动停止做种，不是"下载完成后固定做种这么久"。
          </p>
        </div>

        <p class="hint muted">改动需要重启应用后生效。</p>
      </div>
    </section>

    <section v-show="activeTab === 'archive-ai'">
      <h3>归档</h3>
      <div class="card form">
        <div class="field">
          <label>归档根目录</label>
          <div class="dir">
            <span class="path">{{ form.archiveRoot || "默认归档目录" }}</span>
            <button class="btn btn-ghost small" @click="changeArchiveRoot">更改</button>
          </div>
          <p v-if="archiveRootWarningScope" class="hint warn">
            ⚠️ 这个目录在共享范围「{{
              archiveRootWarningScope.kind === "inbox"
                ? "收到的"
                : archiveRootWarningScope.localPath
            }}」内，归档到这里的文件会被同步给完全信任设备。
          </p>
        </div>

        <div class="field row">
          <label>下载完成后自动归档</label>
          <label class="switch">
            <input type="checkbox" v-model="form.archiveAutoEnabled" />
            <span class="slider"></span>
          </label>
        </div>
        <p class="hint muted">
          总开关；具体归到哪、要不要打标签由「归档」页里逐条规则决定，新规则默认停用。
        </p>
      </div>

      <h3>AI</h3>
      <div class="card form">
        <div class="field">
          <label>模型文件目录</label>
          <div class="dir">
            <span class="path">{{ form.aiModelsDir || "默认模型目录" }}</span>
            <button class="btn btn-ghost small" @click="changeAiModelsDir">更改</button>
          </div>
          <p class="hint muted">
            下载的模型文件归档到这里后会自动出现在「归档」页的模型库分区。
          </p>
        </div>

        <div class="field">
          <label>空闲多久后自动释放内存（分钟）</label>
          <input type="number" min="1" v-model.number="form.aiIdleTimeoutMinutes" />
          <p class="hint muted">
            本地 AI 只在需要时才加载模型，用完这么久没有新请求就自动退出释放内存。
          </p>
        </div>
      </div>
    </section>

    <section v-show="activeTab === 'devices'">
      <div v-if="paired.length" class="card list">
        <div v-for="d in paired" :key="d.id" class="prow">
          <span class="ico">{{ platformIcon(d.platform) }}</span>
          <div class="pinfo">
            <div class="pname">
              <span class="nm">{{ d.name }}</span>
              <span class="online-dot" :class="{ off: !d.online }"></span>
            </div>
            <!-- 信任分级（预览）：我的设备 ⇄ 朋友 -->
            <div class="seg">
              <button
                class="seg-btn"
                :class="{ on: tierOf(d) === 'full' }"
                @click="setTier(d.id, 'full')"
              >
                我的设备
              </button>
              <button
                class="seg-btn"
                :class="{ on: tierOf(d) === 'friend' }"
                @click="setTier(d.id, 'friend')"
              >
                朋友
              </button>
            </div>
          </div>
          <button class="btn btn-danger small" @click="unpair(d.id)">解除配对</button>
        </div>
      </div>
      <div v-else class="empty card muted">还没有已配对的设备。</div>
      <p class="hint muted">
        标记为「我的设备」的，V0.2 起会和本机同步文件（绿/黄/红 状态见「同步」页）；「朋友」只收发、不同步。
      </p>
    </section>

    <div class="save-bar">
      <button class="btn btn-primary" :disabled="settings.saving" @click="save">
        {{ settings.saving ? "保存中…" : "保存设置" }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.settings {
  max-width: 640px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 16px;
}
h3 {
  font-size: 0.85rem;
  margin: 0 0 10px;
}
h3:not(:first-child) {
  margin-top: 26px;
}
.form {
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.field.row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
}
label {
  font-size: 0.88rem;
  font-weight: 600;
}
input[type="text"],
input[type="number"] {
  padding: 9px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.9rem;
}
.dir {
  display: flex;
  align-items: center;
  gap: 10px;
}
.path {
  flex: 1;
  font-size: 0.85rem;
  color: var(--aa-text-dim);
  word-break: break-all;
}
.small {
  padding: 5px 12px;
  min-height: 32px;
  font-size: 0.8rem;
}
.save-bar {
  display: flex;
  justify-content: flex-end;
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--aa-border);
}
.lock {
  font-size: 0.78rem;
  text-align: right;
  margin: 0;
}
.list {
  padding: 4px 0;
}
.prow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
}
.prow + .prow {
  border-top: 1px solid var(--aa-border);
}
.pinfo {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.pname {
  display: flex;
  align-items: center;
  gap: 7px;
}
.nm {
  font-weight: 600;
  font-size: 0.9rem;
}
.tag {
  font-size: 0.68rem;
  font-weight: 600;
  color: #9a6a00;
  background: #ffedcc;
  padding: 1px 8px;
  border-radius: 999px;
  vertical-align: middle;
}
@media (prefers-color-scheme: dark) {
  .tag {
    color: #ffce80;
    background: #4a3a16;
  }
}
.seg {
  display: inline-flex;
  border: 1px solid var(--aa-border);
  border-radius: 999px;
  overflow: hidden;
  width: fit-content;
}
.seg-btn {
  font-size: 0.76rem;
  padding: 4px 12px;
  color: var(--aa-text-dim);
}
.seg-btn.on {
  background: var(--aa-primary);
  color: #fff;
  font-weight: 600;
}
.hint {
  font-size: 0.78rem;
  line-height: 1.6;
  margin: 10px 2px 0;
}
.hint.warn {
  color: #9a6a00;
}
@media (prefers-color-scheme: dark) {
  .hint.warn {
    color: #ffce80;
  }
}
.empty {
  padding: 24px;
  text-align: center;
}

/* 开关 */
.switch {
  position: relative;
  width: 44px;
  height: 24px;
  flex-shrink: 0;
}
.switch input {
  opacity: 0;
  width: 0;
  height: 0;
}
.slider {
  position: absolute;
  inset: 0;
  background: var(--aa-border);
  border-radius: 24px;
  transition: background 0.15s;
}
.slider::before {
  content: "";
  position: absolute;
  height: 18px;
  width: 18px;
  left: 3px;
  top: 3px;
  background: #fff;
  border-radius: 50%;
  transition: transform 0.15s;
}
.switch input:checked + .slider {
  background: var(--aa-primary);
}
.switch input:checked + .slider::before {
  transform: translateX(20px);
}
</style>
