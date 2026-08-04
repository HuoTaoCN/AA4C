// Vitest 全局setup：组件测试跑在 jsdom 里，没有真实 Tauri 运行时。所有
// Tauri Command 调用都收拢在 src/lib/api.ts 的 `api` 对象上（见该文件头注释），
// 各测试文件按需 `vi.mock("../lib/api")` 精确控制返回值；这里只兜底 Tauri 插件
// 的原始导入不报错（`@tauri-apps/api/webview` 的 `getCurrentWebview()` 在
// SendPage.vue 的 onMounted 里无条件调用，不 stub 会在每个用到它的组件测试里
// 崩），具体行为仍由各测试文件的 vi.mock 决定。
import { vi } from "vitest";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));
