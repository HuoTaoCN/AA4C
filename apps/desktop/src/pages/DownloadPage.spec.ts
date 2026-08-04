import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import DownloadPage from "./DownloadPage.vue";
import type { LiveDownloadTask } from "../stores/download";

const listDownloads = vi.fn().mockResolvedValue([]);
const addDownload = vi.fn().mockResolvedValue("d1");
const pauseAllDownloads = vi.fn().mockResolvedValue(0);
const resumeAllDownloads = vi.fn().mockResolvedValue(0);
const clearCompletedDownloads = vi.fn().mockResolvedValue(0);
vi.mock("../lib/api", () => ({
  api: {
    listDownloads: (...a: unknown[]) => listDownloads(...a),
    addDownload: (...a: unknown[]) => addDownload(...a),
    pauseAllDownloads: (...a: unknown[]) => pauseAllDownloads(...a),
    resumeAllDownloads: (...a: unknown[]) => resumeAllDownloads(...a),
    clearCompletedDownloads: (...a: unknown[]) => clearCompletedDownloads(...a),
  },
  asCommandError: (e: unknown) => ({ code: "unknown", message: String(e) }),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn() }));

function task(overrides: Partial<LiveDownloadTask> = {}): LiveDownloadTask {
  return {
    id: "d1",
    kind: "http",
    url: "https://example.com/file.zip",
    savePath: null,
    status: "active",
    totalBytes: 1000,
    downloadedBytes: 0,
    error: null,
    createdAt: Date.now(),
    speedBps: 0,
    ...overrides,
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  listDownloads.mockClear().mockResolvedValue([]);
  addDownload.mockClear().mockResolvedValue("d1");
  pauseAllDownloads.mockClear().mockResolvedValue(0);
  resumeAllDownloads.mockClear().mockResolvedValue(0);
  clearCompletedDownloads.mockClear().mockResolvedValue(0);
});

describe("DownloadPage", () => {
  it("没有任务时显示空态", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.text()).toContain("还没有下载任务。");
  });

  it("空链接不触发添加", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("input").setValue("   ");
    await wrapper.find("button.btn-primary").trigger("click");
    expect(addDownload).not.toHaveBeenCalled();
  });

  it("输入链接点击开始下载后清空输入框、调用 api.addDownload", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("input").setValue("https://example.com/a.zip");
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledWith("https://example.com/a.zip");
    expect((wrapper.find("input").element as HTMLInputElement).value).toBe("");
  });

  it("按 Enter 键等价于点击开始下载", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("input").setValue("https://example.com/b.zip");
    await wrapper.find("input").trigger("keyup.enter");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledWith("https://example.com/b.zip");
  });

  it("只有 active/waiting 任务时只显示「全部暂停」", async () => {
    listDownloads.mockResolvedValue([task({ status: "active" })]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.text()).toContain("全部暂停");
    expect(wrapper.text()).not.toContain("全部继续");
    expect(wrapper.text()).not.toContain("清除已完成");
  });

  it("只有已暂停任务时只显示「全部继续」", async () => {
    listDownloads.mockResolvedValue([task({ status: "paused" })]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.text()).toContain("全部继续");
    expect(wrapper.text()).not.toContain("全部暂停");
  });

  it("有已完成任务时显示「清除已完成」，点击调用 store", async () => {
    listDownloads.mockResolvedValue([task({ status: "complete", downloadedBytes: 1000 })]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    const buttons = wrapper.findAll("button.btn-ghost.small");
    const clearBtn = buttons.find((b) => b.text() === "清除已完成");
    expect(clearBtn).toBeTruthy();
    await clearBtn!.trigger("click");
    expect(clearCompletedDownloads).toHaveBeenCalled();
  });

  it("任务列表渲染对应数量的 DownloadCard", async () => {
    listDownloads.mockResolvedValue([
      task({ id: "d1" }),
      task({ id: "d2", url: "https://example.com/c.zip" }),
    ]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.findAll(".drow")).toHaveLength(2);
  });
});
