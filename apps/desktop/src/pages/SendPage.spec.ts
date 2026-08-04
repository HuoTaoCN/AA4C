import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { createRouter, createWebHistory } from "vue-router";
import SendPage from "./SendPage.vue";
import { useDeviceStore } from "../stores/devices";
import type { DeviceInfo } from "../lib/types";

const sendFiles = vi.fn().mockResolvedValue("task-1");
vi.mock("../lib/api", () => ({
  api: { sendFiles: (...args: unknown[]) => sendFiles(...args) },
  asCommandError: (e: unknown) => ({ code: "unknown", message: String(e) }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

// 覆盖 tests/setup.ts 里的全局 stub：这里要拿到注册的回调，才能在测试里模拟
// 一次真实的拖拽落盘事件（SendPage 的文件列表只有拖拽/对话框两个入口，都是
// Tauri 原生交互，模拟回调是唯一能在 jsdom 里驱动到这条路径的办法）。
let dragDropCallback: ((event: { payload: { type: string; paths: string[] } }) => void) | null =
  null;
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (cb: typeof dragDropCallback) => {
      dragDropCallback = cb;
      return Promise.resolve(() => {});
    },
  }),
}));

function device(overrides: Partial<DeviceInfo> = {}): DeviceInfo {
  return {
    id: "d1",
    name: "小明的电脑",
    platform: "macos",
    version: "0.5.0",
    addr: null,
    online: true,
    trusted: true,
    trustLevel: "friend",
    ...overrides,
  };
}

async function mountSendPage() {
  const router = createRouter({ history: createWebHistory(), routes: [] });
  router.push("/send");
  await router.isReady();
  const wrapper = mount(SendPage, { global: { plugins: [router] } });
  await flushPromises();
  return wrapper;
}

function flushPromises() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function drop(paths: string[]) {
  dragDropCallback?.({ payload: { type: "drop", paths } });
}

beforeEach(() => {
  setActivePinia(createPinia());
  sendFiles.mockClear();
  dragDropCallback = null;
});

describe("SendPage", () => {
  it("没有在线设备时显示空态提示", async () => {
    const wrapper = await mountSendPage();
    expect(wrapper.text()).toContain("附近没有在线设备");
  });

  it("离线设备不出现在设备列表里", async () => {
    const devices = useDeviceStore();
    devices.upsert(device({ id: "offline", online: false }));
    const wrapper = await mountSendPage();
    expect(wrapper.text()).not.toContain("小明的电脑");
  });

  it("未配对设备显示「先配对」而不是可选中状态", async () => {
    const devices = useDeviceStore();
    devices.upsert(device({ trusted: false }));
    const wrapper = await mountSendPage();
    expect(wrapper.text()).toContain("先配对");
  });

  it("没有选文件、没有选设备时 AA 按钮禁用", async () => {
    const devices = useDeviceStore();
    devices.upsert(device());
    const wrapper = await mountSendPage();
    const aaBtn = wrapper.find("button.aa");
    expect((aaBtn.element as HTMLButtonElement).disabled).toBe(true);
  });

  it("拖拽落盘后文件出现在列表里，显示项数", async () => {
    const wrapper = await mountSendPage();
    drop(["/tmp/a.txt", "/tmp/b.txt"]);
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".files li")).toHaveLength(2);
    expect(wrapper.text()).toContain("共 2 项");
  });

  it("点击移除按钮后文件从列表中消失", async () => {
    const wrapper = await mountSendPage();
    drop(["/tmp/a.txt", "/tmp/b.txt"]);
    await wrapper.vm.$nextTick();
    await wrapper.findAll(".rm")[0].trigger("click");
    expect(wrapper.findAll(".files li")).toHaveLength(1);
  });

  it("选中设备 + 有文件后 AA 按钮启用，点击调用 api.sendFiles 并清空文件列表", async () => {
    const devices = useDeviceStore();
    devices.upsert(device());
    const wrapper = await mountSendPage();
    drop(["/tmp/a.txt"]);
    await wrapper.find(".drow").trigger("click");
    await wrapper.vm.$nextTick();

    const aaBtn = wrapper.find("button.aa");
    expect((aaBtn.element as HTMLButtonElement).disabled).toBe(false);
    await aaBtn.trigger("click");
    await flushPromises();

    expect(sendFiles).toHaveBeenCalledWith("d1", ["/tmp/a.txt"]);
    expect(wrapper.find(".files").exists()).toBe(false);
  });

  it("未配对设备即使被点击也不会被选中，AA 按钮保持禁用", async () => {
    const devices = useDeviceStore();
    devices.upsert(device({ trusted: false }));
    const wrapper = await mountSendPage();
    drop(["/tmp/a.txt"]);
    await wrapper.find(".drow").trigger("click");
    await wrapper.vm.$nextTick();
    const aaBtn = wrapper.find("button.aa");
    expect((aaBtn.element as HTMLButtonElement).disabled).toBe(true);
  });
});
