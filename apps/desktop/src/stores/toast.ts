// 轻量应用内 toast（UI_DESIGN_SPEC §4.4 / §6）。

import { defineStore } from "pinia";

export interface Toast {
  id: number;
  kind: "success" | "error" | "info";
  message: string;
  /** 点击时打开的文件夹路径（成功收文件时用）。 */
  openDir?: string;
}

let seq = 0;

export const useToastStore = defineStore("toast", {
  state: (): { items: Toast[] } => ({ items: [] }),
  actions: {
    push(kind: Toast["kind"], message: string, openDir?: string) {
      const id = ++seq;
      this.items.push({ id, kind, message, openDir });
      // 自动消失（错误停留更久）
      const ttl = kind === "error" ? 6000 : 4000;
      setTimeout(() => this.dismiss(id), ttl);
    },
    dismiss(id: number) {
      this.items = this.items.filter((t) => t.id !== id);
    },
  },
});
