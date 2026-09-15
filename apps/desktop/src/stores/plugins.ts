// 插件 store（V0.8「Focus」F1/F3）：本次构建装了哪些插件。
//
// 前端不再硬编码「有下载和归档这两个能力」——那是 `nav.ts` 里 `CAPABILITIES` 写死
// 五大能力的老做法，直接后果是空插件构建也会显示两个点不动的入口。
// 现在问后端要 `plugin_manifest`，装了什么就显示什么。

import { defineStore } from "pinia";
import { api, type PluginInfo } from "../lib/api";
import { PLUGIN_PAGES, type NavItem } from "../lib/nav";

export const usePluginStore = defineStore("plugins", {
  state: (): { installed: PluginInfo[] } => ({ installed: [] }),

  getters: {
    /** 「更多」分区要显示的导航项。
     *
     * 只列在 `PLUGIN_PAGES` 里登记过页面的插件——将来第三方插件可能只提供设置项、
     * 没有自己的页面，那种不该在导航里占位。 */
    navItems: (s): NavItem[] =>
      s.installed
        .map((p) => PLUGIN_PAGES[p.id])
        .filter((it): it is NavItem => Boolean(it)),
  },

  actions: {
    async load() {
      this.installed = await api.pluginManifest();
    },
    /** 某个插件装了没有——页面自己也要能问，好在没装时给出「这个构建没有这个能力」
     * 而不是一堆调用失败。 */
    has(id: string): boolean {
      return this.installed.some((p) => p.id === id);
    },
  },
});
