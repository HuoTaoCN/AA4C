<script setup lang="ts">
// 设备页 —— 首页（UI_DESIGN_SPEC §3.1）。
//
// 这一屏要回答的唯一问题：**我的设备现在连上了吗，怎么连的，连不上该怎么办。**
//
// F3 之前首页是一个「能力」宫格，五个格子链到侧栏已有的五项——菜单的菜单。
// 那是产品对「用户先做什么」没有意见的症状，也让 AA4C 唯一不可替代的能力
// （跨网自组网）在界面上完全没有落点。
import { computed, onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useDeviceStore } from "../stores/devices";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { platformIcon } from "../lib/format";
import DeviceCard from "../components/DeviceCard.vue";
import DeviceRow from "../components/DeviceRow.vue";
import { Card, EmptyState, StatusDot, Toolbar } from "../components/ui";

const devices = useDeviceStore();
const toast = useToastStore();
const { self, pendingIntroductions } = storeToRefs(devices);

/** 已配对的设备——这一屏的主体。 */
const paired = computed(() =>
  devices.devices.filter((d) => d.trusted && d.id !== self.value?.id),
);

/** 局域网里发现、但还没配对的。放在下面，是次要内容。 */
const nearby = computed(() =>
  devices.devices.filter((d) => !d.trusted && d.online && d.id !== self.value?.id),
);

onMounted(() => {
  void devices.loadReachability();
  void devices.loadPendingIntroductions();
});

async function toggleTrust(id: string, full: boolean) {
  try {
    await devices.setTrustLevel(id, full ? "full" : "friend");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function confirmIntro(id: string, name: string) {
  try {
    await devices.confirmIntroduction(id);
    toast.push("success", `已把「${name}」标记为你的设备`);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function dismissIntro(id: string) {
  try {
    await devices.dismissIntroduction(id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="devices">
    <Toolbar title="设备" subtitle="你的设备现在连上了吗、怎么连的" />

    <!-- 本机 -->
    <Card v-if="self" class="selfcard">
      <div class="self">
        <span class="ico">{{ platformIcon(self.platform) }}</span>
        <div>
          <div class="nm">{{ self.name }}</div>
          <StatusDot tone="ok" label="这台设备 · 在线" />
        </div>
      </div>
    </Card>

    <!-- 待确认的引荐：有才显示（TRUST_DESIGN §5.8） -->
    <section v-if="pendingIntroductions.length">
      <h2>待确认的设备</h2>
      <Card padding="none">
        <div v-for="p in pendingIntroductions" :key="p.deviceId" class="intro">
          <span class="ico">{{ platformIcon(p.platform) }}</span>
          <div class="text">
            <div class="nm">{{ p.name }}</div>
            <p class="hint">
              你的「{{ p.introducedByName ?? "已移除的设备" }}」说这也是你的设备。
              确认前请核对指纹与对方设备上显示的是否一致。
            </p>
            <code class="fp">{{ p.deviceId }}</code>
          </div>
          <div class="acts">
            <button class="b primary" @click="confirmIntro(p.deviceId, p.name)">
              标记为我的设备
            </button>
            <button class="b" @click="dismissIntro(p.deviceId)">忽略</button>
          </div>
        </div>
      </Card>
    </section>

    <!-- 我的设备 -->
    <section>
      <h2>我的设备</h2>
      <Card v-if="paired.length" padding="none">
        <DeviceRow
          v-for="d in paired"
          :key="d.id"
          :device="d"
          :reach="devices.reachOf(d.id)"
          @toggle-trust="toggleTrust"
        />
      </Card>
      <EmptyState
        v-else
        title="还没有连接任何设备。"
        hint="在另一台设备上打开 AA连接，连到同一个 WiFi，它就会出现在下面。"
      />
    </section>

    <!-- 附近可配对 -->
    <section v-if="nearby.length">
      <h2>附近可配对</h2>
      <div class="grid">
        <DeviceCard v-for="d in nearby" :key="d.id" :device="d" />
      </div>
    </section>
  </div>
</template>

<style scoped>
.devices {
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
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
.self {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}
.ico {
  font-size: var(--fs-xl);
  flex: none;
}
.nm {
  font-size: var(--fs-base);
  font-weight: var(--fw-medium);
  margin-bottom: var(--sp-1);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: var(--sp-3);
}

/* 引荐行：比普通设备行多一段指纹与两个按钮 */
.intro {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
}
.intro + .intro {
  border-top: 1px solid var(--aa-border);
}
.text {
  flex: 1;
  min-width: 0;
}
.hint {
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
  line-height: 1.5;
}
.fp {
  display: block;
  font-size: var(--fs-sm);
  word-break: break-all;
  color: var(--aa-text-dim);
}
.acts {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  flex: none;
}
.b {
  padding: var(--sp-1) var(--sp-3);
  border-radius: var(--aa-radius-sm);
  border: 1px solid var(--aa-border);
  font-size: var(--fs-sm);
  white-space: nowrap;
}
.b.primary {
  background: var(--aa-primary);
  border-color: var(--aa-primary);
  color: #fff;
}
</style>
