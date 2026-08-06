import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import DownloadCard from "./DownloadCard.vue";
import { useDownloadStore, type LiveDownloadTask } from "../stores/download";

const openPath = vi.fn().mockResolvedValue(undefined);
const revealItemInDir = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: (p: string) => openPath(p),
  revealItemInDir: (p: string) => revealItemInDir(p),
}));
const writeText = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: (t: string) => writeText(t),
}));
vi.mock("../lib/api", () => ({
  api: {
    pauseDownload: vi.fn().mockResolvedValue(undefined),
    resumeDownload: vi.fn().mockResolvedValue(undefined),
    cancelDownload: vi.fn().mockResolvedValue(undefined),
    retryDownload: vi.fn().mockResolvedValue("d2"),
    listDownloads: vi.fn().mockResolvedValue([]),
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
  revealItemInDir.mockClear();
  writeText.mockClear();
  vi.spyOn(window, "confirm").mockReturnValue(true);
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

  it("完成态显示打开文件/打开文件夹，不显示取消/重试", () => {
    const wrapper = mount(
      DownloadCard,
      { props: { task: task({ status: "complete", savePath: "/tmp/file.zip", downloadedBytes: 1000 }) } },
    );
    expect(wrapper.find('button[title="打开文件"]').exists()).toBe(true);
    expect(wrapper.find('button[title="打开所在文件夹"]').exists()).toBe(true);
    expect(wrapper.find('button[title="重试"]').exists()).toBe(false);
    expect(wrapper.find(".danger").exists()).toBe(false);
  });

  it("点击打开文件调用 openPath(savePath)", async () => {
    const wrapper = mount(DownloadCard, {
      props: { task: task({ status: "complete", savePath: "/tmp/file.zip" }) },
    });
    await wrapper.find('button[title="打开文件"]').trigger("click");
    expect(openPath).toHaveBeenCalledWith("/tmp/file.zip");
  });

  it("点击打开所在文件夹调用 revealItemInDir(savePath)，不是 openPath", async () => {
    const wrapper = mount(DownloadCard, {
      props: { task: task({ status: "complete", savePath: "/tmp/file.zip" }) },
    });
    await wrapper.find('button[title="打开所在文件夹"]').trigger("click");
    expect(revealItemInDir).toHaveBeenCalledWith("/tmp/file.zip");
    expect(openPath).not.toHaveBeenCalled();
  });

  it("错误态显示错误文案、重试按钮，进度条标记 error class", () => {
    const wrapper = mount(DownloadCard, {
      props: { task: task({ status: "error", error: "资源不存在" }) },
    });
    expect(wrapper.text()).toContain("资源不存在");
    expect(wrapper.find(".bar i").classes()).toContain("error");
    expect(wrapper.find('button[title="重试"]').exists()).toBe(true);
  });

  it("点击重试调用 store.retry", async () => {
    const download = useDownloadStore();
    const retrySpy = vi.spyOn(download, "retry");
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "error", error: "x" }) } });
    await wrapper.find('button[title="重试"]').trigger("click");
    expect(retrySpy).toHaveBeenCalledWith("d1");
  });

  it("任何状态都能复制下载链接", async () => {
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    await wrapper.find('button[title="复制下载链接"]').trigger("click");
    expect(writeText).toHaveBeenCalledWith("https://example.com/file.zip");
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
    await wrapper.find('button[title="取消"]').trigger("click");
    expect(cancelSpy).toHaveBeenCalledWith("d1");
  });

  it("取消并删除文件需要二次确认，确认后带 deleteLocal=true 调用 store.cancel", async () => {
    const download = useDownloadStore();
    const cancelSpy = vi.spyOn(download, "cancel");
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    await wrapper.find('button[title="取消并删除文件"]').trigger("click");
    expect(window.confirm).toHaveBeenCalled();
    expect(cancelSpy).toHaveBeenCalledWith("d1", true);
  });

  it("取消确认弹窗点了取消时不调用 store.cancel", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const download = useDownloadStore();
    const cancelSpy = vi.spyOn(download, "cancel");
    const wrapper = mount(DownloadCard, { props: { task: task({ status: "active" }) } });
    await wrapper.find('button[title="取消并删除文件"]').trigger("click");
    expect(cancelSpy).not.toHaveBeenCalled();
  });
});
