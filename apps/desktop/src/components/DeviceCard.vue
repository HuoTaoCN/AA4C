<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { usePairingStore } from "../stores/pairing";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { platformIcon } from "../lib/format";
import type { DeviceInfo } from "../lib/types";

const props = defineProps<{ device: DeviceInfo }>();
const router = useRouter();
const pairing = usePairingStore();
const toast = useToastStore();

const statusLine = computed(() => {
  const online = props.device.online ? "在线" : "离线";
  const paired = props.device.trusted ? "已配对" : "未配对";
  return `${online} · ${paired}`;
});

function goSend() {
  router.push({ name: "send", query: { device: props.device.id } });
}

async function pair() {
  try {
    await pairing.start(props.device);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="device card" :class="{ offline: !device.online }">
    <div class="icon">{{ platformIcon(device.platform) }}</div>
    <div class="name">{{ device.name }}</div>
    <div class="status">
      <span class="online-dot" :class="{ off: !device.online }"></span>
      {{ statusLine }}
    </div>

    <button
      v-if="device.online && device.trusted"
      class="btn btn-primary act"
      @click="goSend"
    >
      AA 文件
    </button>
    <button
      v-else-if="device.online && !device.trusted"
      class="btn btn-ghost act"
      @click="pair"
    >
      配对
    </button>
    <button v-else class="btn act" disabled>离线</button>
  </div>
</template>

<style scoped>
.device {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 7px;
  padding: 18px 14px;
  text-align: center;
}
.device.offline {
  opacity: 0.55;
}
.icon {
  font-size: 2rem;
}
.name {
  font-weight: 700;
  font-size: 0.98rem;
}
.status {
  font-size: 0.8rem;
  color: var(--aa-text-dim);
  display: flex;
  align-items: center;
  gap: 6px;
}
.act {
  margin-top: 6px;
  width: 100%;
}
</style>
