<script setup lang="ts">
import { computed, reactive, watchEffect } from "vue";
import { useDeviceStore } from "../stores/devices";
import { useSettingsStore } from "../stores/settings";
import { useToastStore } from "../stores/toast";
import { api, asCommandError } from "../lib/api";
import { platformIcon } from "../lib/format";
import type { Settings } from "../lib/types";

const devices = useDeviceStore();
const settings = useSettingsStore();
const toast = useToastStore();

// 本地编辑副本，从 store 同步
const form = reactive<Settings>({
  deviceName: "",
  saveDir: "",
  autoAcceptFromTrusted: false,
  listenPort: 42420,
});
watchEffect(() => {
  if (settings.settings) Object.assign(form, settings.settings);
});

const paired = computed(() => devices.devices.filter((d) => d.trusted));

// 信任分级（预览）：后端 trust_level 尚未实现（见 SYNC_DESIGN.md），
// 这里仅本地演示交互，配对设备默认「朋友」，可标记为「我的设备」。
type Tier = "full" | "friend";
const tierPreview = reactive<Record<string, Tier>>({});
const tierOf = (id: string): Tier => tierPreview[id] ?? "friend";
function setTier(id: string, t: Tier) {
  tierPreview[id] = t;
  toast.push(
    "info",
    t === "full"
      ? "已标记为「我的设备」——同步功能 V0.2 上线后将参与跨设备文件同步"
      : "已设为「朋友」（预览）",
  );
}

async function changeDir() {
  const picked = await settings.pickSaveDir();
  if (picked) form.saveDir = picked;
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

      <div class="actions">
        <button class="btn btn-primary" :disabled="settings.saving" @click="save">
          {{ settings.saving ? "保存中…" : "保存" }}
        </button>
      </div>
      <p class="lock muted">🔒 所有传输均已加密</p>
    </div>

    <h3>已配对设备 <span class="tag">含信任分级预览</span></h3>
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
              :class="{ on: tierOf(d.id) === 'full' }"
              @click="setTier(d.id, 'full')"
            >
              我的设备
            </button>
            <button
              class="seg-btn"
              :class="{ on: tierOf(d.id) === 'friend' }"
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
  margin: 26px 0 10px;
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
input[type="text"] {
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
.actions {
  display: flex;
  justify-content: flex-end;
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
