<script setup lang="ts">
// 设备页的一行（UI_DESIGN_SPEC §3.1）。
//
// 这是 V0.8「Focus」整轮重构最终要落到的那个像素：AA4C 唯一不可替代的能力是
// 「我的几台设备在任何网络下自己连成一片，不用注册账号」，而在 F2 之前，
// 连接阶梯每 30 秒算出一次结果就被丢掉，界面上只有「在线 / 离线」两个词。
// 这一行把它说出来——连上了没有、走的哪一档、没连上该怎么办。
import { computed } from "vue";
import { useRouter } from "vue-router";
import { Button, Icon, ListRow } from "./ui";
import {
  connectionViaText,
  connectionViaTone,
  platformIcon,
  reachFailureText,
  reachStateText,
  timeText,
} from "../lib/format";
import type { DeviceInfo, DeviceReachability } from "../lib/types";

const props = defineProps<{
  device: DeviceInfo;
  /** 没探测过时是 `undefined`——**按「还不知道」处理，不要替它编一个「离线」**。 */
  reach?: DeviceReachability;
}>();
const emit = defineEmits<{ toggleTrust: [id: string, full: boolean] }>();

const router = useRouter();

const isMine = computed(() => props.device.trustLevel === "full");
const state = computed(() => props.reach?.state ?? "unknown");

/** 右侧那行字：「已连接 · 局域网直连」/「连不上」/「正在检查…」。 */
const statusText = computed(() => {
  const base = reachStateText(state.value);
  const via = connectionViaText(props.reach?.via);
  return state.value === "reachable" && via ? `${base} · ${via}` : base;
});

const statusTone = computed(() => {
  if (state.value === "unreachable") return "danger" as const;
  if (state.value === "unknown") return "muted" as const;
  // 连上了：走中继值得提醒一句（慢，而且要有中转站）
  return connectionViaTone(props.reach?.via) === "warn"
    ? ("warn" as const)
    : ("ok" as const);
});

const subtitle = computed(() => {
  const trust = isMine.value ? "我的设备" : "朋友";
  // 连上了就显示「几点连上的」；没连上就显示「上次是什么时候、怎么连上的」——
  // 后者在排查时比前者有用得多。
  if (state.value === "reachable" && props.reach?.lastOkAt) {
    return `${trust} · ${timeText(props.reach.lastOkAt)}`;
  }
  if (state.value === "unreachable" && props.reach?.lastOkAt) {
    const via = connectionViaText(props.reach.via);
    const when = timeText(props.reach.lastOkAt);
    return via ? `${trust} · 上次 ${when} 经${via}连上` : `${trust} · 上次 ${when} 连上`;
  }
  return trust;
});

/** 连不上时的原因 + 下一步。规格要求：**不能只说坏了**（§3.1 / §6）。 */
const failure = computed(() =>
  state.value === "unreachable" ? reachFailureText(props.reach?.reason) : "",
);

/** 只有「不在同一个网络而且远程连接没开」这一种能一键去处理。 */
const canGoSettings = computed(
  () => props.reach?.reason === "not_on_lan_and_remote_off",
);

function send() {
  void router.push({ path: "/send", query: { to: props.device.id } });
}
</script>

<template>
  <ListRow
    :title="device.name"
    :subtitle="subtitle"
    :status="statusText"
    :tone="statusTone"
  >
    <template #icon>{{ platformIcon(device.platform) }}</template>

    <template #detail>
      <p v-if="failure" class="why">{{ failure }}</p>
    </template>

    <template #actions>
      <Button
        v-if="canGoSettings"
        size="sm"
        variant="ghost"
        @click="router.push('/settings')"
      >
        去设置
      </Button>
      <Button size="sm" variant="ghost" @click="send">
        <Icon name="send" :size="14" /> 发送
      </Button>
      <!-- 可逆操作放这儿；解除配对（不可逆）在设置页，见 UI_DESIGN_SPEC §3.5 -->
      <Button
        size="sm"
        :variant="isMine ? 'plain' : 'ghost'"
        :title="
          isMine
            ? '取消后它的文件会从同步视图里消失，本机已下载的不动'
            : '标为我的设备后才参与跨设备同步'
        "
        @click="emit('toggleTrust', device.id, !isMine)"
      >
        <Icon v-if="isMine" name="check" :size="14" /> 我的设备
      </Button>
    </template>
  </ListRow>
</template>

<style scoped>
.why {
  margin: var(--sp-2) 0 0;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
  line-height: 1.5;
}
</style>
