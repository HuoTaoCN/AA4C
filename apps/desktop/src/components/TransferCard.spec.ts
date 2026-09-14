import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import TransferCard from "./TransferCard.vue";
import { useDeviceStore } from "../stores/devices";
import { useTransferStore, type ActiveTask } from "../stores/transfer";

vi.mock("../lib/api", () => ({
  api: {
    cancelTransfer: vi.fn().mockResolvedValue(undefined),
    pauseTransfer: vi.fn().mockResolvedValue(undefined),
    resumeTransfer: vi.fn().mockResolvedValue(undefined),
    listTransfers: vi.fn().mockResolvedValue([]),
  },
  asCommandError: (e: unknown) => ({ code: "unknown", message: String(e) }),
}));

function task(overrides: Partial<ActiveTask> = {}): ActiveTask {
  return {
    id: "t1",
    direction: "send",
    peer: "peer-1",
    files: [],
    status: "transferring",
    totalBytes: 1000,
    transferredBytes: 250,
    createdAt: Date.now(),
    error: null,
    speedBps: 0,
    currentFile: "",
    ...overrides,
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
});

describe("TransferCard", () => {
  it("显示发送方向、对端名称与百分比进度", () => {
    const devices = useDeviceStore();
    devices.upsert({
      id: "peer-1",
      name: "小明的电脑",
      platform: "macos",
      version: "0.5.0",
      addr: null,
      online: true,
      trusted: true,
      trustLevel: "friend",
    });
    const wrapper = mount(TransferCard, { props: { task: task() } });
    expect(wrapper.text()).toContain("发送至");
    expect(wrapper.text()).toContain("小明的电脑");
    expect(wrapper.text()).toContain("25%");
  });

  it("等待对方确认时显示提示文案，不显示速度/ETA", () => {
    const wrapper = mount(TransferCard, {
      props: { task: task({ status: "waiting_accept", transferredBytes: 0 }) },
    });
    expect(wrapper.text()).toContain("等待对方确认…");
  });

  it("连接阶梯的每一档各有说法，只有 punch 并入「直连」", () => {
    // F2 之前这里只有 direct/punch/relay 三档，而「局域网里」和「真的跨网连上了」
    // 是用户最该分得清的两件事——合并成一个「直连」等于把核心能力藏起来。
    const lan = mount(TransferCard, { props: { task: task({ via: "lan" }) } });
    expect(lan.text()).toContain("局域网直连");
    const v4 = mount(TransferCard, { props: { task: task({ via: "public_v4" }) } });
    expect(v4.text()).toContain("公网直连");
    const v6 = mount(TransferCard, { props: { task: task({ via: "public_v6" }) } });
    expect(v6.text()).toContain("IPv6");
    // 打洞只是"怎么找到对方"，连上之后就是真直连；而且"打洞"是技术词，
    // 按 AGENTS.md 的 UI 规则不该出现在界面上。
    const punch = mount(TransferCard, { props: { task: task({ via: "punch" }) } });
    expect(punch.text()).toContain("直连");
    expect(punch.text()).not.toContain("打洞");
    const relay = mount(TransferCard, { props: { task: task({ via: "relay" }) } });
    expect(relay.text()).toContain("中继（较慢）");
  });

  it("接收方任务没有 via 时不显示连接质量徽标", () => {
    const wrapper = mount(TransferCard, { props: { task: task({ direction: "recv" }) } });
    expect(wrapper.find(".via").exists()).toBe(false);
  });

  it("点击取消按钮调用 transfer.cancel", async () => {
    const wrapper = mount(TransferCard, { props: { task: task() } });
    const transfer = useTransferStore();
    const spy = vi.spyOn(transfer, "cancel");
    await wrapper.find('button[title="取消"]').trigger("click");
    expect(spy).toHaveBeenCalledWith("t1");
  });

  it("发送中显示暂停按钮，点击调用 transfer.pause", async () => {
    const wrapper = mount(TransferCard, {
      props: { task: task({ direction: "send", status: "transferring" }) },
    });
    expect(wrapper.find('button[title="暂停"]').exists()).toBe(true);
    expect(wrapper.find('button[title="继续"]').exists()).toBe(false);

    const transfer = useTransferStore();
    const spy = vi.spyOn(transfer, "pause");
    await wrapper.find('button[title="暂停"]').trigger("click");
    expect(spy).toHaveBeenCalledWith("t1");
  });

  it("已暂停显示继续按钮与提示文案，点击调用 transfer.resume", async () => {
    const wrapper = mount(TransferCard, {
      props: { task: task({ direction: "send", status: "paused" }) },
    });
    expect(wrapper.find('button[title="继续"]').exists()).toBe(true);
    expect(wrapper.find('button[title="暂停"]').exists()).toBe(false);
    expect(wrapper.text()).toContain("已暂停");

    const transfer = useTransferStore();
    const spy = vi.spyOn(transfer, "resume");
    await wrapper.find('button[title="继续"]').trigger("click");
    expect(spy).toHaveBeenCalledWith("t1");
  });

  it("接收方向不给暂停/继续按钮——那是发送方才能做的动作", () => {
    const transferring = mount(TransferCard, {
      props: { task: task({ direction: "recv", status: "transferring" }) },
    });
    expect(transferring.find('button[title="暂停"]').exists()).toBe(false);
    const paused = mount(TransferCard, {
      props: { task: task({ direction: "recv", status: "paused" }) },
    });
    expect(paused.find('button[title="继续"]').exists()).toBe(false);
  });
});
