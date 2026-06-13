// 设置 store（UI_DESIGN_SPEC §3.4 / §9）。

import { defineStore } from "pinia";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import type { Settings } from "../lib/types";

interface State {
  settings: Settings | null;
  saving: boolean;
}

export const useSettingsStore = defineStore("settings", {
  state: (): State => ({ settings: null, saving: false }),

  actions: {
    async load() {
      this.settings = await api.getSettings();
    },
    async save(next: Settings) {
      this.saving = true;
      try {
        await api.updateSettings(next);
        this.settings = next;
      } finally {
        this.saving = false;
      }
    },
    /** 打开系统目录选择器，返回选中的保存目录（取消返回 null）。 */
    async pickSaveDir(): Promise<string | null> {
      const picked = await open({ directory: true, multiple: false });
      return typeof picked === "string" ? picked : null;
    },
  },
});
