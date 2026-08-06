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
const addTorrentFile = vi.fn().mockResolvedValue("t1");
vi.mock("../lib/api", () => ({
  api: {
    listDownloads: (...a: unknown[]) => listDownloads(...a),
    addDownload: (...a: unknown[]) => addDownload(...a),
    addTorrentFile: (...a: unknown[]) => addTorrentFile(...a),
    pauseAllDownloads: (...a: unknown[]) => pauseAllDownloads(...a),
    resumeAllDownloads: (...a: unknown[]) => resumeAllDownloads(...a),
    clearCompletedDownloads: (...a: unknown[]) => clearCompletedDownloads(...a),
  },
  asCommandError: (e: unknown) => ({ code: "unknown", message: String(e) }),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), revealItemInDir: vi.fn() }));
const readText = vi.fn().mockResolvedValue("");
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: () => readText(),
  writeText: vi.fn().mockResolvedValue(undefined),
}));
const dialogOpen = vi.fn().mockResolvedValue(null);
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...a: unknown[]) => dialogOpen(...a),
}));

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
  readText.mockClear().mockResolvedValue("");
  addTorrentFile.mockClear().mockResolvedValue("t1");
  dialogOpen.mockClear().mockResolvedValue(null);
});

describe("DownloadPage", () => {
  it("没有任务时显示空态", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.text()).toContain("还没有下载任务。");
  });

  it("空链接不触发添加", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("textarea").setValue("   ");
    await wrapper.find("button.btn-primary").trigger("click");
    expect(addDownload).not.toHaveBeenCalled();
  });

  it("输入链接点击开始下载后清空输入框、调用 api.addDownload", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("textarea").setValue("https://example.com/a.zip");
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledWith("https://example.com/a.zip", undefined);
    expect((wrapper.find("textarea").element as HTMLTextAreaElement).value).toBe("");
  });

  it("按 Enter 键等价于点击开始下载", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("textarea").setValue("https://example.com/b.zip");
    await wrapper.find("textarea").trigger("keydown.enter");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledWith("https://example.com/b.zip", undefined);
  });

  it("多行输入批量添加，跳过批内重复链接，弹出汇总 toast", async () => {
    const wrapper = mount(DownloadPage);
    await wrapper.find("textarea").setValue(
      "https://example.com/a.zip\nhttps://example.com/b.zip\nhttps://example.com/a.zip",
    );
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledTimes(2);
    expect(addDownload).toHaveBeenCalledWith("https://example.com/a.zip", undefined);
    expect(addDownload).toHaveBeenCalledWith("https://example.com/b.zip", undefined);
  });

  it("链接已在下载列表中时跳过，不重复添加", async () => {
    listDownloads.mockResolvedValue([task({ url: "https://example.com/a.zip" })]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    await wrapper.find("textarea").setValue("https://example.com/a.zip");
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();
    expect(addDownload).not.toHaveBeenCalled();
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

  it("筛选 tab 只显示对应状态的任务", async () => {
    listDownloads.mockResolvedValue([
      task({ id: "d1", status: "active" }),
      task({ id: "d2", status: "complete", url: "https://example.com/c.zip" }),
      task({ id: "d3", status: "error", url: "https://example.com/e.zip", error: "x" }),
    ]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.findAll(".drow")).toHaveLength(3);

    const tabs = wrapper.findAll(".tab");
    await tabs.find((t) => t.text() === "已完成")!.trigger("click");
    expect(wrapper.findAll(".drow")).toHaveLength(1);
    expect(wrapper.text()).toContain("c.zip");

    await tabs.find((t) => t.text() === "失败")!.trigger("click");
    expect(wrapper.findAll(".drow")).toHaveLength(1);
    expect(wrapper.text()).toContain("e.zip");
  });

  it("搜索框按标题过滤任务", async () => {
    listDownloads.mockResolvedValue([
      task({ id: "d1", url: "https://example.com/apple.zip" }),
      task({ id: "d2", url: "https://example.com/banana.zip" }),
    ]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    await wrapper.find("input.search").setValue("apple");
    expect(wrapper.findAll(".drow")).toHaveLength(1);
    expect(wrapper.text()).toContain("apple.zip");
  });

  it("有进行中任务时显示总速度统计条", async () => {
    listDownloads.mockResolvedValue([task({ status: "active", speedBps: 1024 })]);
    const wrapper = mount(DownloadPage);
    await flushPromises();
    expect(wrapper.find(".stats").text()).toContain("1 个进行中");
  });

  it("高级选项默认折叠，点击后展开", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    // 断言 v-show 真正的效果（display 是否被置为 none）而不是 VTU 的
    // `isVisible()`——后者在未挂到 document 的 wrapper 上判定不可靠（实测
    // 展开后 style 已经是空字符串、它仍返回 false）。要测的是"点一下会不会展开"。
    expect(wrapper.find(".advanced").attributes("style")).toContain("display: none");
    const toggle = wrapper.findAll(".link-btn").find((b) => b.text().includes("高级选项"));
    await toggle!.trigger("click");
    expect(wrapper.find(".advanced").attributes("style")).not.toContain("display: none");
  });

  it("填了高级选项后添加，选项随 addDownload 一起传给后端", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    const toggle = wrapper.findAll(".link-btn").find((b) => b.text().includes("高级选项"));
    await toggle!.trigger("click");

    const inputs = wrapper.findAll(".afield input");
    await inputs[0].setValue("renamed.zip"); // 另存为文件名
    await inputs[1].setValue("https://example.com/from"); // Referer
    await inputs[2].setValue("sid=1"); // Cookie

    await wrapper.find("textarea").setValue("https://example.com/a.zip");
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();

    expect(addDownload).toHaveBeenCalledWith("https://example.com/a.zip", {
      out: "renamed.zip",
      referer: "https://example.com/from",
      cookie: "sid=1",
    });
  });

  it("批量添加时忽略自定义文件名（多个任务不能同名）", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    const toggle = wrapper.findAll(".link-btn").find((b) => b.text().includes("高级选项"));
    await toggle!.trigger("click");
    await wrapper.findAll(".afield input")[0].setValue("renamed.zip");

    await wrapper
      .find("textarea")
      .setValue("https://example.com/a.zip\nhttps://example.com/b.zip");
    await wrapper.find("button.btn-primary").trigger("click");
    await flushPromises();

    expect(addDownload).toHaveBeenCalledTimes(2);
    for (const call of addDownload.mock.calls) {
      expect(call[1]?.out).toBeUndefined();
    }
  });

  it("选择种子文件后调用 addTorrentFile；取消选择时什么都不做", async () => {
    const wrapper = mount(DownloadPage);
    await flushPromises();
    const btn = wrapper.findAll(".link-btn").find((b) => b.text().includes("种子文件"));

    // 用户在文件选择器里点了取消
    dialogOpen.mockResolvedValue(null);
    await btn!.trigger("click");
    await flushPromises();
    expect(addTorrentFile).not.toHaveBeenCalled();

    dialogOpen.mockResolvedValue("/tmp/movie.torrent");
    await btn!.trigger("click");
    await flushPromises();
    expect(addTorrentFile).toHaveBeenCalledWith("/tmp/movie.torrent", undefined);
  });

  it("剪贴板出现新链接时弹出检测提示，点击添加下载后清除提示", async () => {
    vi.useFakeTimers();
    readText.mockResolvedValue("https://example.com/clip.zip");
    const wrapper = mount(DownloadPage);
    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(wrapper.text()).toContain("检测到剪贴板里有个下载链接");

    await wrapper.find(".detect-actions .btn-primary").trigger("click");
    await flushPromises();
    expect(addDownload).toHaveBeenCalledWith("https://example.com/clip.zip", undefined);
    expect(wrapper.find(".detect").exists()).toBe(false);
    vi.useRealTimers();
  });

  it("剪贴板检测提示点击忽略后不添加，也不再重复弹出同一条", async () => {
    vi.useFakeTimers();
    readText.mockResolvedValue("https://example.com/clip2.zip");
    const wrapper = mount(DownloadPage);
    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(wrapper.find(".detect").exists()).toBe(true);

    await wrapper.find(".detect-actions .btn-ghost").trigger("click");
    await flushPromises();
    expect(wrapper.find(".detect").exists()).toBe(false);
    expect(addDownload).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(wrapper.find(".detect").exists()).toBe(false);
    vi.useRealTimers();
  });
});
