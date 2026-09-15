<script setup lang="ts">
// 线性图标（UI_DESIGN_SPEC §5.4）。
//
// 规范从 V0.2 就写着「单色线性图标（lucide），平台图标可用 emoji 占位，V0.2 替换」，
// 到 V0.7 还是 9 个 emoji、图标库连依赖都没加——V0.8 F3 兑现。
//
// **按名字从一张白名单里取**，而不是让各页自己 `import { Send } from "@lucide/vue"`：
// lucide 有 6000+ 个图标，逐页自由取用很快就会出现「同一个动作在两页用了两个不同图标」。
// 要用新图标就在这里登记一个，顺带也是一次「真的需要新图标吗」的自检。
//
// **平台图标是例外，仍用 emoji**（💻🖥📱🗄）——它们的辨识度高于任何线性图标，见 §5.4。
import {
  Check,
  ChevronRight,
  Copy,
  Download,
  FolderOpen,
  FolderTree,
  Globe,
  Link,
  MonitorSmartphone,
  Pause,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Send,
  Server,
  Settings,
  ShieldCheck,
  Trash2,
  Wifi,
  X,
} from "@lucide/vue";

const REGISTRY = {
  check: Check,
  "chevron-right": ChevronRight,
  copy: Copy,
  download: Download,
  "folder-open": FolderOpen,
  "folder-tree": FolderTree,
  globe: Globe,
  link: Link,
  devices: MonitorSmartphone,
  pause: Pause,
  play: Play,
  plus: Plus,
  refresh: RefreshCw,
  retry: RotateCw,
  send: Send,
  server: Server,
  settings: Settings,
  shield: ShieldCheck,
  trash: Trash2,
  wifi: Wifi,
  x: X,
} as const;

export type IconName = keyof typeof REGISTRY;

const props = withDefaults(defineProps<{ name: IconName; size?: number }>(), {
  size: 18,
});
</script>

<template>
  <component :is="REGISTRY[props.name]" :size="props.size" :stroke-width="1.75" />
</template>
