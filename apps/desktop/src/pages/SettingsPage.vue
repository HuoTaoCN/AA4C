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

    <h3>已配对设备</h3>
    <div v-if="paired.length" class="card list">
      <div v-for="d in paired" :key="d.id" class="prow">
        <span class="ico">{{ platformIcon(d.platform) }}</span>
        <span class="nm">{{ d.name }}</span>
        <span class="online-dot" :class="{ off: !d.online }"></span>
        <button class="btn btn-danger small" @click="unpair(d.id)">解除配对</button>
      </div>
    </div>
    <div v-else class="empty card muted">还没有已配对的设备。</div>
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
  padding: 10px 16px;
}
.prow + .prow {
  border-top: 1px solid var(--aa-border);
}
.nm {
  flex: 1;
  font-weight: 600;
  font-size: 0.9rem;
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
