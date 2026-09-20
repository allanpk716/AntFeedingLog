import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import ToastHost from "./ToastHost.vue";
import { clearToasts, showError, showSuccess } from "../lib/toast";

// 展示层测试：store 用真实现（行为在 toast.test.ts 验过），这里只验接线——
// 档位类名、原因上屏、× 提前关、自动消失联动。

beforeEach(() => {
  clearToasts();
});

describe("ToastHost（轻提示宿主，保湿方式+轻提示票 03）", () => {
  it("无提示时宿主照常挂载，不出任何条目", () => {
    const wrapper = mount(ToastHost);

    expect(wrapper.find(".toast-host").exists()).toBe(true);
    expect(wrapper.findAll(".toast-item").length).toBe(0);
  });

  it("成功条目挂成功档类名；失败条目挂失败档类名且主文案与原因都上屏", async () => {
    const wrapper = mount(ToastHost);
    showSuccess("已保存");
    showError("备份失败", "磁盘没有空间");
    await nextTick();

    const items = wrapper.findAll(".toast-item");
    expect(items.length).toBe(2);
    expect(items[0]!.classes()).toContain("toast-success");
    expect(items[0]!.text()).toContain("已保存");
    expect(items[1]!.classes()).toContain("toast-error");
    expect(items[1]!.text()).toContain("备份失败");
    expect(items[1]!.find(".toast-reason").text()).toBe("磁盘没有空间");
  });

  it("同屏至多 3 条由 store 收口，宿主照实渲染", async () => {
    const wrapper = mount(ToastHost);
    showSuccess("第 1 条");
    showSuccess("第 2 条");
    showSuccess("第 3 条");
    showSuccess("第 4 条");
    await nextTick();

    const items = wrapper.findAll(".toast-item");
    expect(items.length).toBe(3);
    expect(items[0]!.text()).toContain("第 2 条");
  });

  it("点 × 提前关：对应条目立刻消失，其余不受影响", async () => {
    const wrapper = mount(ToastHost);
    showError("备份失败", "磁盘没有空间");
    showSuccess("已保存");
    await nextTick();

    await wrapper.findAll(".toast-item")[0]!.find(".toast-close").trigger("click");
    await nextTick();

    const items = wrapper.findAll(".toast-item");
    expect(items.length).toBe(1);
    expect(items[0]!.text()).toContain("已保存");
  });

  it("store 自动消失后宿主条目跟着消失（响应式联动）", async () => {
    // 只 fake setTimeout/clearTimeout：默认全套 fake 连 setImmediate 一起劫持，
    // nextTick 的微任务调度会挂死（仓库先例 LogListPage.test.ts）
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const wrapper = mount(ToastHost);
      showSuccess("已保存");
      await nextTick();
      expect(wrapper.findAll(".toast-item").length).toBe(1);

      await vi.advanceTimersByTimeAsync(2500);
      await nextTick();
      expect(wrapper.findAll(".toast-item").length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
