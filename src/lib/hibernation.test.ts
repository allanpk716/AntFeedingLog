import { describe, expect, it } from "vitest";
import type { HibernationPreview } from "../types";
import { defaultExpectedEnd, hibernationBanner } from "./hibernation";

const seg: HibernationPreview = {
  id: 1,
  start_date: "2026-08-20",
  expected_end_date: "2026-09-25",
};

describe("冬眠横幅展示（票 05，视觉文案对齐 mock-a-light）", () => {
  it("主文案：入眠日 / 预计出眠 / 剩余天数（还有 N 天）", () => {
    expect(hibernationBanner(seg, "2026-09-18")).toEqual({
      line: "❄ 冬眠中 · 08-20 入眠 · 预计出眠 09-25（还有 7 天）",
      remainingDays: 7,
      nearWake: true,
    });
  });

  it("未进临近窗口（默认 7 天外）：nearWake=false，不显角标", () => {
    const view = hibernationBanner(
      { id: 1, start_date: "2026-08-20", expected_end_date: "2026-09-26" },
      "2026-09-18",
    );
    expect(view.remainingDays).toBe(8);
    expect(view.nearWake).toBe(false);
  });

  it("预计出眠当天：文案切「今天预计出眠」，仍在临近窗口", () => {
    const view = hibernationBanner(seg, "2026-09-25");
    expect(view.line).toContain("（今天预计出眠）");
    expect(view.remainingDays).toBe(0);
    expect(view.nearWake).toBe(true);
  });

  it("预计日已过仍冬眠：文案提示已过 N 天，角标持续显示", () => {
    const view = hibernationBanner(seg, "2026-09-27");
    expect(view.line).toContain("（预计日已过 2 天）");
    expect(view.remainingDays).toBe(-2);
    expect(view.nearWake).toBe(true);
  });

  it("临近窗口天数可配置（设置项 wake_remind_days_ahead 接入点）", () => {
    expect(hibernationBanner(seg, "2026-09-18", 3).nearWake).toBe(false);
    expect(hibernationBanner(seg, "2026-09-22", 3).nearWake).toBe(true);
  });
});

describe("默认预计出眠日", () => {
  it("开始日 + 120 天（票 05 预填口径）", () => {
    expect(defaultExpectedEnd("2026-12-01")).toBe("2027-03-31");
    expect(defaultExpectedEnd("2024-11-01")).toBe("2025-03-01");
  });
});
