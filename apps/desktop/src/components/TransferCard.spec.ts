import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import TransferCard from "./TransferCard.vue";
import { useDeviceStore } from "../stores/devices";
import { useTransferStore, type ActiveTask } from "../stores/transfer";

vi.mock("../lib/api", () => ({
  api: { cancelTransfer: vi.fn().mockResolvedValue(undefined) },
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

  it("direct/punch 都归并显示为「直连」，relay 显示「中继（较慢）」", () => {
    const direct = mount(TransferCard, { props: { task: task({ via: "direct" }) } });
    expect(direct.text()).toContain("直连");
    const punch = mount(TransferCard, { props: { task: task({ via: "punch" }) } });
    expect(punch.text()).toContain("直连");
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
    await wrapper.find(".cancel").trigger("click");
    expect(spy).toHaveBeenCalledWith("t1");
  });
});
