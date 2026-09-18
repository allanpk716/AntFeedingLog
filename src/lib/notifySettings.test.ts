import { describe, expect, it } from "vitest";
import type { AppSettings } from "../types";
import { DAYS_AHEAD_ERROR, parseDaysAhead, toForm, toSettings } from "./notifySettings";

const settings: AppSettings = {
  notify_master_enabled: true,
  notify_overdue_enabled: false,
  notify_hibernation_enabled: true,
  wake_remind_days_ahead: 7,
  autostart_enabled: true,
};

describe("通知设置：设置态 ↔ 表单态", () => {
  it("toForm 回显开关与提前天数文本", () => {
    expect(toForm(settings)).toEqual({
      master: true,
      overdue: false,
      hibernation: true,
      daysAheadText: "7",
    });
  });

  it("toSettings 带上开关与提前天数，autostart 原样透传", () => {
    const out = toSettings(
      { master: false, overdue: true, hibernation: false, daysAheadText: "3" },
      true,
    );
    expect(out).toEqual({
      notify_master_enabled: false,
      notify_overdue_enabled: true,
      notify_hibernation_enabled: false,
      wake_remind_days_ahead: 3,
      autostart_enabled: true,
    });
  });

  it("toForm → toSettings 往返保持设置（autostart 除外）", () => {
    const out = toSettings(toForm(settings), settings.autostart_enabled);
    expect(out).toEqual(settings);
  });
});

describe("通知设置：提前天数校验", () => {
  it("非负整数 0–365 合法（0 = 出眠日当天才临近提醒）", () => {
    expect(parseDaysAhead("0")).toBe(0);
    expect(parseDaysAhead("7")).toBe(7);
    expect(parseDaysAhead(" 14 ")).toBe(14);
    expect(parseDaysAhead(3)).toBe(3); // number 输入框经 v-model 可能给回数字
    expect(parseDaysAhead("365")).toBe(365);
  });

  it("负数、小数、空串、非数字、超 365 非法", () => {
    expect(parseDaysAhead("-1")).toBeNull();
    expect(parseDaysAhead("1.5")).toBeNull();
    expect(parseDaysAhead("")).toBeNull();
    expect(parseDaysAhead("abc")).toBeNull();
    expect(parseDaysAhead("366")).toBeNull();
  });

  it("非法提前天数时 toSettings 返回 null（不落库）", () => {
    const out = toSettings(
      { master: true, overdue: true, hibernation: true, daysAheadText: "-1" },
      true,
    );
    expect(out).toBeNull();
    expect(DAYS_AHEAD_ERROR).toContain("0–365");
  });
});
