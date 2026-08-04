<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRoute } from "vue-router";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import { useDeviceStore } from "../stores/devices";
import { usePairingStore } from "../stores/pairing";
import { useTransferStore } from "../stores/transfer";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { baseName, platformIcon } from "../lib/format";
import type { DeviceInfo } from "../lib/types";

const route = useRoute();
const devices = useDeviceStore();
const pairing = usePairingStore();
const transfer = useTransferStore();
const toast = useToastStore();

const paths = ref<string[]>([]);
const dragging = ref(false);
const selectedId = ref<string | null>(
  typeof route.query.device === "string" ? route.query.device : null,
);

const onlineDevices = computed(() => devices.visible.filter((d) => d.online));
const selected = computed(() =>
  onlineDevices.value.find((d) => d.id === selectedId.value) ?? null,
);
const canSend = computed(
  () => paths.value.length > 0 && !!selected.value?.trusted,
);

let unlistenDrop: UnlistenFn | null = null;

onMounted(async () => {
  unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "over" || p.type === "enter") dragging.value = true;
    else if (p.type === "drop") {
      dragging.value = false;
      addPaths(p.paths);
    } else dragging.value = false;
  });
});
onUnmounted(() => unlistenDrop?.());

function addPaths(incoming: string[]) {
  const set = new Set(paths.value);
  for (const p of incoming) set.add(p);
  paths.value = [...set];
}

async function pickFiles() {
  const picked = await open({ multiple: true });
  if (Array.isArray(picked)) addPaths(picked);
  else if (typeof picked === "string") addPaths([picked]);
}
async function pickFolder() {
  const picked = await open({ directory: true });
  if (typeof picked === "string") addPaths([picked]);
}
function removePath(p: string) {
  paths.value = paths.value.filter((x) => x !== p);
}

function pick(device: DeviceInfo) {
  if (device.trusted) selectedId.value = device.id;
}
async function pair(device: DeviceInfo) {
  try {
    await pairing.start(device);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function aa() {
  if (!canSend.value || !selected.value) return;
  try {
    await transfer.send(selected.value, paths.value);
    paths.value = [];
    toast.push("info", "正在发送…");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="send">
    <h2>AA 发送</h2>
    <div class="steps">
      <!-- 第 1 步：选文件 -->
      <section class="step">
        <div class="label">1 · 选文件</div>
        <div
          class="drop card"
          :class="{ active: dragging }"
          @click="pickFiles"
        >
          <template v-if="paths.length === 0">
            <p>{{ dragging ? "松开，把文件 AA 出去" : "拖文件到这里" }}</p>
            <p class="muted">或点击选择</p>
          </template>
          <ul v-else class="files" @click.stop>
            <li v-for="p in paths" :key="p">
              <span class="fn">{{ baseName(p) }}</span>
              <button class="rm" @click="removePath(p)">✕</button>
            </li>
          </ul>
        </div>
        <div class="pickers">
          <button class="btn btn-ghost" @click="pickFiles">选择文件</button>
          <button class="btn btn-ghost" @click="pickFolder">选择文件夹</button>
          <span v-if="paths.length" class="muted count">
            共 {{ paths.length }} 项
          </span>
        </div>
      </section>

      <!-- 第 2 步：选设备 -->
      <section class="step">
        <div class="label">2 · 选设备</div>
        <div v-if="onlineDevices.length" class="devices card">
          <div
            v-for="d in onlineDevices"
            :key="d.id"
            class="drow"
            :class="{ sel: d.id === selectedId, dim: !d.trusted }"
            @click="pick(d)"
          >
            <span class="ico">{{ platformIcon(d.platform) }}</span>
            <span class="nm">{{ d.name }}</span>
            <span class="online-dot"></span>
            <button v-if="!d.trusted" class="btn btn-ghost small" @click.stop="pair(d)">
              先配对
            </button>
            <span v-else-if="d.id === selectedId" class="check">✓</span>
          </div>
        </div>
        <div v-else class="empty card muted">附近没有在线设备</div>
      </section>

      <!-- 第 3 步：AA -->
      <section class="step center">
        <div class="label">3 · AA</div>
        <button class="aa" :disabled="!canSend" @click="aa">AA！</button>
      </section>
    </div>
  </div>
</template>

<style scoped>
.send {
  max-width: 980px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 16px;
}
.steps {
  display: grid;
  /* 裸 `1fr` 轨道的隐式最小宽度是内容的 min-content，不是 0——设备名一旦不换行
     （见下面 .nm 的单行省略号截断），未截断的整串文本会把这个轨道的 min-content
     撑到自己的全长，级联把整个三栏布局顶爆出横向滚动。`minmax(0, 1fr)` 才是
     "占 1fr 份额，但允许缩到 0"的写法，让轨道宽度真正由 1fr 分配决定，
     子元素内部的 overflow/ellipsis 才有意义。 */
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
  gap: 18px;
  align-items: start;
}
.label {
  font-size: 0.82rem;
  color: var(--aa-text-dim);
  margin-bottom: 8px;
  font-weight: 600;
}
.drop {
  min-height: 150px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  cursor: pointer;
  border-style: dashed;
  padding: 14px;
}
.drop.active {
  border-color: var(--aa-primary);
  background: var(--aa-primary-dim);
}
.drop p {
  margin: 3px 0;
}
.files {
  list-style: none;
  margin: 0;
  padding: 0;
  width: 100%;
  max-height: 150px;
  overflow-y: auto;
}
.files li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 5px 4px;
  font-size: 0.85rem;
}
.fn {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rm {
  color: var(--aa-text-dim);
}
.rm:hover {
  color: var(--aa-danger);
}
.pickers {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}
.count {
  font-size: 0.8rem;
}
.devices {
  padding: 6px;
}
.drow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--aa-radius-sm);
  cursor: pointer;
}
.drow:hover {
  background: var(--aa-surface-2);
}
.drow.sel {
  background: var(--aa-primary-dim);
}
.drow.dim {
  cursor: default;
}
.nm {
  flex: 1;
  min-width: 0;
  font-weight: 600;
  font-size: 0.9rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.check {
  color: var(--aa-primary);
  font-weight: 800;
}
.small {
  padding: 4px 10px;
  min-height: 30px;
  font-size: 0.8rem;
}
.center {
  align-self: stretch;
  display: flex;
  flex-direction: column;
  align-items: center;
}
.aa {
  width: 96px;
  height: 96px;
  border-radius: 50%;
  background: var(--aa-primary);
  color: #fff;
  font-size: 1.5rem;
  font-weight: 800;
  box-shadow: 0 8px 24px var(--aa-primary-dim);
  transition: transform 0.1s, filter 0.15s;
}
.aa:hover:not(:disabled) {
  filter: brightness(1.05);
}
.aa:active:not(:disabled) {
  transform: scale(0.96);
}
.aa:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.empty {
  padding: 24px;
  text-align: center;
}

@media (max-width: 700px) {
  .steps {
    grid-template-columns: 1fr;
  }
}
</style>
