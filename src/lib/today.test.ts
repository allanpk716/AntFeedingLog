import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { watch } from "vue";
import { startTodayClock, stopTodayClock, syncToday, todayIsoRef } from "./today";

// 仓库先例（LogListPage.test.ts）：只 fake 需要的计时器，不劫持 setImmediate
// （flushPromises 会挂死）。本模块测试额外 fake Date——todayIso 读系统时钟。
function useClockFakes() {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
}

beforeEach(() => {
  useClockFakes();
});

afterEach(() => {
  stopTodayClock();
  vi.useRealTimers();
});

describe("全局今天时钟源", () => {
  it("syncToday 对表到系统时钟：日期变了才写 ref", () => {
    vi.setSystemTime(new Date(2026, 8, 21, 10, 0));
    syncToday();
    expect(todayIsoRef().value).toBe("2026-09-21");

    // 同一天内再对表：值不变
    vi.setSystemTime(new Date(2026, 8, 21, 23, 59));
    syncToday();
    expect(todayIsoRef().value).toBe("2026-09-21");

    vi.setSystemTime(new Date(2026, 8, 22, 0, 0, 30));
    syncToday();
    expect(todayIsoRef().value).toBe("2026-09-22");
  });

  it("每分钟 tick 跨零点自动追上新的一天", () => {
    vi.setSystemTime(new Date(2026, 8, 21, 23, 59, 30));
    startTodayClock(); // 启动即对表一次
    expect(todayIsoRef().value).toBe("2026-09-21");

    // 30 秒后跨过 00:00，下一次分钟 tick（60s 周期）落在零点后
    vi.setSystemTime(new Date(2026, 8, 22, 0, 0, 20));
    vi.advanceTimersByTime(60_000);
    expect(todayIsoRef().value).toBe("2026-09-22");
  });

  it("同日 tick 静默：日期没变不写 ref（不触发任何下游重渲染）", () => {
    vi.setSystemTime(new Date(2026, 8, 21, 12, 0));
    startTodayClock();
    let fires = 0;
    const stop = watch(todayIsoRef(), () => {
      fires += 1;
    });

    vi.advanceTimersByTime(60_000 * 5); // 同一天内 tick 五次
    expect(fires).toBe(0);
    stop();
  });

  it("窗口 focus 兜底：睡眠唤醒/后台标签页错过 tick，切回即追上（跨多天也追上）", () => {
    vi.setSystemTime(new Date(2026, 8, 19, 9, 0));
    startTodayClock();

    // 睡了三天醒来，不进任何 tick
    vi.setSystemTime(new Date(2026, 8, 22, 8, 0));
    window.dispatchEvent(new Event("focus"));
    expect(todayIsoRef().value).toBe("2026-09-22");
  });

  it("visibilitychange 兜底：标签页切回前台即追上", () => {
    vi.setSystemTime(new Date(2026, 8, 21, 22, 0));
    startTodayClock();

    vi.setSystemTime(new Date(2026, 8, 22, 7, 30));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(todayIsoRef().value).toBe("2026-09-22");
  });

  it("stopTodayClock 停表清监听：此后 tick 与兜底事件都不再动 ref；重复启停不炸", () => {
    vi.setSystemTime(new Date(2026, 8, 21, 12, 0));
    startTodayClock();
    startTodayClock(); // 幂等
    stopTodayClock();
    stopTodayClock(); // 幂等

    vi.setSystemTime(new Date(2026, 8, 23, 12, 0));
    vi.advanceTimersByTime(60_000 * 10);
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(todayIsoRef().value).toBe("2026-09-21"); // 停表后定格
  });
});
