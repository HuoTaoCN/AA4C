<script setup lang="ts">
import { computed } from "vue";
import { usePairingStore } from "../stores/pairing";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";

const pairing = usePairingStore();
const toast = useToastStore();

const pinSession = computed(() => pairing.pinSession);
const requestSession = computed(() => pairing.requestSession);
const trustPrompt = computed(() => pairing.trustPrompt);

function resolveTrust(tier: "full" | "friend") {
  const name = pairing.trustPrompt?.peerName ?? "对方设备";
  pairing.resolveTrust(tier);
  toast.push(
    "success",
    tier === "full"
      ? `已把「${name}」设为你的设备——同步功能 V0.2 上线后将参与跨设备文件同步`
      : `已和「${name}」配对（朋友）`,
  );
}

// 确认码分两组（3+3）便于目视比对
const pinGroups = computed(() => {
  const pin = pinSession.value?.pin ?? "";
  return [pin.slice(0, 3), pin.slice(3, 6)];
});

async function confirm(sessionId: string, accept: boolean) {
  try {
    await pairing.confirm(sessionId, accept);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <!-- 配对成功后：信任分级追问 -->
  <div v-if="trustPrompt" class="overlay">
    <div class="dialog card">
      <h3>配对成功 🎉</h3>
      <p class="sub">
        和 <b>{{ trustPrompt.peerName }}</b> 配对成功。<br />这是你自己的设备吗？
      </p>
      <p class="note muted">
        是 → 之后可与本机同步文件；不是 → 仅用于收发，不同步。
      </p>
      <div class="actions">
        <button class="btn btn-ghost" @click="resolveTrust('friend')">
          不是，朋友
        </button>
        <button class="btn btn-primary" @click="resolveTrust('full')">
          是，我的设备
        </button>
      </div>
    </div>
  </div>

  <div v-else-if="pinSession || requestSession" class="overlay">
    <!-- 确认码比对（优先） -->
    <div v-if="pinSession" class="dialog card">
      <h3>确认码</h3>
      <p class="sub">和 {{ pinSession.peerName }} 的屏幕对一下，数字一样吗？</p>
      <div class="pin">
        <span class="group">{{ pinGroups[0] }}</span>
        <span class="group">{{ pinGroups[1] }}</span>
      </div>
      <div class="actions">
        <button class="btn btn-ghost" @click="confirm(pinSession.sessionId, false)">
          不一致
        </button>
        <button class="btn btn-primary" @click="confirm(pinSession.sessionId, true)">
          一致，配对
        </button>
      </div>
    </div>

    <!-- 接受配对请求 -->
    <div v-else-if="requestSession" class="dialog card">
      <h3>配对请求</h3>
      <p class="sub">
        <b>{{ requestSession.peerName }}</b> 想与本机配对
      </p>
      <div class="actions">
        <button class="btn btn-ghost" @click="confirm(requestSession.sessionId, false)">
          拒绝
        </button>
        <button class="btn btn-primary" @click="confirm(requestSession.sessionId, true)">
          接受
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 60;
}
.dialog {
  width: 340px;
  padding: 24px;
  text-align: center;
}
h3 {
  margin: 0 0 6px;
}
.sub {
  margin: 0 0 18px;
  color: var(--aa-text-dim);
  font-size: 0.9rem;
}
.note {
  margin: -8px 0 18px;
  font-size: 0.78rem;
  line-height: 1.5;
}
.pin {
  display: flex;
  justify-content: center;
  gap: 18px;
  margin-bottom: 22px;
}
.group {
  font-size: 2.6rem;
  font-weight: 800;
  letter-spacing: 0.18em;
  font-variant-numeric: tabular-nums;
  color: var(--aa-primary);
}
.actions {
  display: flex;
  gap: 10px;
}
.actions .btn {
  flex: 1;
}
</style>
