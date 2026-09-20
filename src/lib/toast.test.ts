import { beforeEach, describe, expect, it, vi } from "vitest";
import { clearToasts, dismissToast, showError, showSuccess, toastItems } from "./toast";

// 只 fake setTimeout/clearTimeout（仓库先例 LogListPage.test.ts）：默认全套 fake
// 会连 setImmediate 一起劫持，flushPromises 内部走 setImmediate 会挂死。
// 本文件是纯 store 行为测试，直接驱动假时钟即可，不需要组件挂载。

beforeEach(() => {
  clearToasts();
});

describe("toast store（轻提示，保湿方式+轻提示票 03）", () => {
  it("showSuccess 入列成功档：kind=success、主文案、无原因", () => {
    showSuccess("已保存");

    expect(toastItems.value.length).toBe(1);
    expect(toastItems.value[0]!.kind).toBe("success");
    expect(toastItems.value[0]!.message).toBe("已保存");
    expect(toastItems.value[0]!.reason).toBe("");
  });

  it("showError 入列失败档：kind=error、主文案带原因", () => {
    showError("备份失败", "磁盘没有空间");

    expect(toastItems.value.length).toBe(1);
    expect(toastItems.value[0]!.kind).toBe("error");
    expect(toastItems.value[0]!.message).toBe("备份失败");
    expect(toastItems.value[0]!.reason).toBe("磁盘没有空间");
  });

  it("成功约 2.5 秒自动消失（2.5s 前在，到点即走）", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      showSuccess("已保存");
      vi.advanceTimersByTime(2499);
      expect(toastItems.value.length).toBe(1);
      vi.advanceTimersByTime(1);
      expect(toastItems.value.length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("失败约 5 秒自动消失（比成功档活得久）", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      showError("备份失败", "磁盘没有空间");
      vi.advanceTimersByTime(2500);
      expect(toastItems.value.length).toBe(1); // 成功档的点它还没走
      vi.advanceTimersByTime(2499);
      expect(toastItems.value.length).toBe(1);
      vi.advanceTimersByTime(1);
      expect(toastItems.value.length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("提前关：dismissToast 立即移除，挂起的自动消失计时一并撤销", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      showError("备份失败", "磁盘没有空间");
      dismissToast(toastItems.value[0]!.id);
      expect(toastItems.value.length).toBe(0);
      // 计时器已撤销：走完原定时长也不会再触发任何变动（不抛错即通过）
      vi.advanceTimersByTime(10_000);
      expect(toastItems.value.length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("同屏至多 3 条：第 4 条入列时最旧的先走", () => {
    showSuccess("第 1 条");
    showSuccess("第 2 条");
    showSuccess("第 3 条");
    showSuccess("第 4 条");

    expect(toastItems.value.length).toBe(3);
    expect(toastItems.value.map((t) => t.message)).toEqual(["第 2 条", "第 3 条", "第 4 条"]);
  });

  it("混合档位各自按时长消失：成功先走，失败留守", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      showSuccess("已保存");
      showError("备份失败", "磁盘没有空间");
      vi.advanceTimersByTime(2500);
      expect(toastItems.value.map((t) => t.kind)).toEqual(["error"]);
      vi.advanceTimersByTime(2500);
      expect(toastItems.value.length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("clearToasts 清空全部并撤销挂起计时（测试复位用）", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      showSuccess("已保存");
      showError("备份失败", "磁盘没有空间");
      clearToasts();
      expect(toastItems.value.length).toBe(0);
      vi.advanceTimersByTime(10_000);
      expect(toastItems.value.length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
