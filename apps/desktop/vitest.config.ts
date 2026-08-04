import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

// 复用 vite.config.ts 同款 Vue 插件；测试跑在 jsdom 里，不需要 Tauri dev server
// 的那些端口/HMR 配置（那部分只在 `vite.config.ts` 里，两份配置刻意不合一，
// 避免测试意外依赖固定端口 1420）。
export default defineConfig({
  plugins: [vue()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./tests/setup.ts"],
  },
});
