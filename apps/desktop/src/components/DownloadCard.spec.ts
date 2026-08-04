import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import DownloadCard from "./DownloadCard.vue";
import { useDownloadStore, type LiveDownloadTask } from "../stores/download";

const openPath = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: (p: string) => openPath(p) }));
vi.mock("../lib/api", () => ({
  api: {
    pauseDownload: vi.fn().mockResolvedValue(undefined),
    resumeDownload: vi.fn().mockResolvedValue(undefined),
    cancelDownload: vi.fn().mockResolvedValue(undefined),
  },
  asCommandError: (e: unknown) => ({ code: "unknown", message: String(e) }),
}));

function task(overrides: Partial<LiveDownloadTask> = {}): LiveDownloadTask {
  return {
    id: "d1",
    kind: "http",
    url: "https://example.com/file.zip",
    savePath: null,
    status: "active",
    totalBytes: 1000,
    downloadedBytes: 400,
    error: null,
    createdAt: Date.now(),
    speedBps: 0,
    ...overrides,
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  openPath.mockClear();
});

describe("DownloadCard", () => {
  it("进行中只显示暂停按钮，不显示继续/打开文件夹", () => {
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    expect(wrapper.find('button[title="暂停"]').exists()).toBe(true);
    expect(wrapper.find('button[title="继续"]').exists()).toBe(false);
    expect(wrapper.find('button[title="打开所在文件夹"]').exists()).toBe(false);
  });

  it("已暂停只显示继续按钮", () => {
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "paused" }) } });
    expect(wrapper.find('button[title="继续"]').exists()).toBe(true);
    expect(wrapper.find('button[title="暂停"]').exists()).toBe(false);
  });

  it("完成态只显示打开文件夹，不显示取消", () => {
    const wrapper = mount(
      DownloadCard,
      { props: { task: task({ status: "complete", savePath: "/tmp/file.zip", downloadedBytes: 1000 }) } },
    );
    expect(wrapper.find('button[title="打开所在文件夹"]').exists()).toBe(true);
    expect(wrapper.find(".danger").exists()).toBe(false);
  });

  it("点击打开文件夹调用 openPath(savePath)", async () => {
    const wrapper = mount(DownloadCard, {
      props: { task: task({ status: "complete", savePath: "/tmp/file.zip" }) },
    });
    await wrapper.find('button[title="打开所在文件夹"]').trigger("click");
    expect(openPath).toHaveBeenCalledWith("/tmp/file.zip");
  });

  it("错误态显示错误文案，进度条标记 error class", () => {
    const wrapper = mount(DownloadCard, {
      props: { task: task({ status: "error", error: "资源不存在" }) },
    });
    expect(wrapper.text()).toContain("资源不存在");
    expect(wrapper.find(".bar i").classes()).toContain("error");
  });

  it("BT 任务进行中显示做种/连接数，HTTP 任务不显示", () => {
    const bt = mount(DownloadCard, {
      props: {
        task: task({ kind: "bt", status: "active", seeders: 5, peers: 2, ratio: 1.5 }),
      },
    });
    expect(bt.text()).toContain("5 做种");
    expect(bt.text()).toContain("2 连接");

    const http = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    expect(http.text()).not.toContain("做种");
  });

  it("点击暂停/继续/取消调用对应 store action", async () => {
    const download = useDownloadStore();
    const pauseSpy = vi.spyOn(download, "pause");
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    await wrapper.find('button[title="暂停"]').trigger("click");
    expect(pauseSpy).toHaveBeenCalledWith("d1");

    const cancelSpy = vi.spyOn(download, "cancel");
    await wrapper.find(".danger").trigger("click");
    expect(cancelSpy).toHaveBeenCalledWith("d1");
  });
});
