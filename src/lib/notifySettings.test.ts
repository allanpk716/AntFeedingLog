import { describe, expect, it } from "vitest";
import { DAYS_AHEAD_ERROR, parseDaysAhead, toForm, toSettings, type NotifySettingsModel } from "./notifySettings";

const settings: NotifySettingsModel = {
  notify_master_enabled: true,
  notify_overdue_enabled: false,
  notify_hibernation_enabled: true,
  notify_retrieval_enabled: true,
  wake_remind_days_ahead: 7,
  autostart_enabled: true,
  pushover_user: "u-库内",
  pushover_token: "t-库内",
};

describe("通知设置：设置态 ↔ 表单态", () => {
  it("toForm 回显总开关、撤食开关、提前天数文本与 Pushover 凭据（作废的分类子开关不进表单）", () => {
    expect(toForm(settings)).toEqual({
      master: true,
      retrieval: true,
      daysAheadText: "7",
      pushoverUser: "u-库内",
      pushoverToken: "t-库内",
    });
  });

  it("toForm 对缺省的撤食键兜底为开（旧 mock / 迁移期数据不炸表单）", () => {
    const legacy: NotifySettingsModel = {
      notify_master_enabled: true,
      notify_overdue_enabled: false,
      notify_hibernation_enabled: true,
      wake_remind_days_ahead: 7,
      autostart_enabled: true,
      pushover_user: "",
      pushover_token: "",
    };
    expect(toForm(legacy).retrieval).toBe(true);
  });

  it("toSettings 带上总开关、撤食开关、提前天数与 Pushover 凭据，作废分类开关固定回写 true，autostart 原样透传", () => {
    const out = toSettings(
      { master: false, retrieval: false, daysAheadText: "3", pushoverUser: "u-新", pushoverToken: "t-新" },
      true,
    );
    expect(out).toEqual({
      notify_master_enabled: false,
      notify_overdue_enabled: true,
      notify_hibernation_enabled: true,
      notify_retrieval_enabled: false,
      wake_remind_days_ahead: 3,
      autostart_enabled: true,
      pushover_user: "u-新",
      pushover_token: "t-新",
    });
  });

  it("toForm → toSettings 往返保持设置（作废分类开关固定回写 true，autostart 除外）", () => {
    const out = toSettings(toForm(settings), settings.autostart_enabled);
    expect(out).toEqual({
      notify_master_enabled: true,
      notify_overdue_enabled: true,
      notify_hibernation_enabled: true,
      notify_retrieval_enabled: true,
      wake_remind_days_ahead: 7,
      autostart_enabled: true,
      pushover_user: "u-库内",
      pushover_token: "t-库内",
    });
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
      { master: true, retrieval: true, daysAheadText: "-1", pushoverUser: "u", pushoverToken: "t" },
      true,
    );
    expect(out).toBeNull();
    expect(DAYS_AHEAD_ERROR).toContain("0–365");
  });
});
