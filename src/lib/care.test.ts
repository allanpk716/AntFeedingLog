import { describe, expect, it } from "vitest";
import type { ColonyAction, FoodTileInfo, RecentLog } from "../types";
import { actionTile, feedingTooltip, formatRecent, isFeeding, nowLocalDateTime } from "./care";

function action(partial: Partial<ColonyAction> & { action_id: number }): ColonyAction {
  return {
    name: `操作${partial.action_id}`,
    icon: null,
    kind: "reminding",
    is_feeding: false,
    suggested_interval_days: 3,
    days_since_last: null,
    overdue: false,
    foods: [],
    ...partial,
  };
}

describe("操作块展示态", () => {
  it("提醒类三态：今天·已记录（绿）/ 距上次 X 天（绿）/ ⚠ 超期 N 天（红）", () => {
    expect(actionTile(action({ action_id: 1, days_since_last: 0 }), false)).toEqual({
      tone: "ok",
      text: "今天 · 已记录",
    });
    expect(actionTile(action({ action_id: 1, days_since_last: 2 }), false)).toEqual({
      tone: "ok",
      text: "距上次 2 天",
    });
    expect(
      actionTile(action({ action_id: 1, days_since_last: 4, overdue: true }), false),
    ).toEqual({ tone: "bad", text: "⚠ 超期 1 天" });
  });

  it("超期天数 = 距上次 − 建议间隔（验收 4：4 天对建议 3 → 超期 1 天）", () => {
    const view = actionTile(
      action({ action_id: 1, days_since_last: 4, suggested_interval_days: 3, overdue: true }),
      false,
    );
    expect(view.text).toBe("⚠ 超期 1 天");
  });

  it("提醒类从未记录：中性提示，不红", () => {
    expect(actionTile(action({ action_id: 1, days_since_last: null }), false)).toEqual({
      tone: "none",
      text: "尚未记录",
    });
  });

  it("登记类中性灰，无论多少天都不红（验收 5）", () => {
    expect(
      actionTile(
        action({ action_id: 2, kind: "log_only", suggested_interval_days: null, days_since_last: 100 }),
        false,
      ),
    ).toEqual({ tone: "reg", text: "距上次 100 天" });
    expect(
      actionTile(action({ action_id: 2, kind: "log_only", days_since_last: 0 }), false),
    ).toEqual({ tone: "reg", text: "今天 · 已记录" });
  });

  it("食物层顶的红：报「该喂X了」，不拿统一层数字硬算（F3 评审：统一层新鲜时会出现负数）", () => {
    const food = (id: number, name: string, overdue: boolean): FoodTileInfo => ({
      food_id: id,
      name,
      suggested_interval_days: 7,
      days_since_last: overdue ? 8 : 1,
      overdue,
    });

    // 面包虫 8 > 7 食物超期、统一层 2 ≤ 3 新鲜 → 整块红但报「该喂面包虫了」
    expect(
      actionTile(
        action({
          action_id: 1,
          is_feeding: true,
          days_since_last: 2,
          suggested_interval_days: 3,
          overdue: true,
          foods: [food(3, "面包虫", true)],
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 该喂面包虫了" });

    // 两种食物都超期：顿号连接
    expect(
      actionTile(
        action({
          action_id: 1,
          is_feeding: true,
          days_since_last: 2,
          suggested_interval_days: 3,
          overdue: true,
          foods: [food(1, "种子", true), food(3, "面包虫", true)],
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 该喂种子、面包虫了" });

    // 统一层自己也超（8 > 3）：维持「超期 N 天」原句式，操作层措辞优先
    expect(
      actionTile(
        action({
          action_id: 1,
          is_feeding: true,
          days_since_last: 8,
          suggested_interval_days: 3,
          overdue: true,
          foods: [food(3, "面包虫", true)],
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 5 天" });
  });

  it("冬眠静音：永不红、文案加（静音）", () => {
    expect(
      actionTile(action({ action_id: 1, days_since_last: 4, overdue: true }), true),
    ).toEqual({ tone: "mute", text: "距上次 4 天（静音）" });
    expect(actionTile(action({ action_id: 1, days_since_last: 0 }), true)).toEqual({
      tone: "mute",
      text: "今天 · 已记录（静音）",
    });
    expect(actionTile(action({ action_id: 1, days_since_last: null }), true)).toEqual({
      tone: "mute",
      text: "尚未记录（静音）",
    });
    expect(
      actionTile(action({ action_id: 2, kind: "log_only", days_since_last: 9 }), true),
    ).toEqual({ tone: "mute", text: "距上次 9 天（静音）" });
  });
});

describe("喂食判定", () => {
  it("按 is_feeding 标记位判定，与名字彻底解耦", () => {
    expect(isFeeding(action({ action_id: 1, name: "喂食", is_feeding: true }))).toBe(true);
    // 名字不叫「喂食」但标记位为真 → 仍弹食物多选
    expect(isFeeding(action({ action_id: 5, name: "投喂", is_feeding: true }))).toBe(true);
    // 名字叫「喂食」但标记位为假 → 不弹
    expect(isFeeding(action({ action_id: 2, name: "喂食", is_feeding: false }))).toBe(false);
    expect(isFeeding(action({ action_id: 3, name: "垃圾清理", is_feeding: false }))).toBe(false);
  });
});

describe("喂食块悬停提示（反馈第二轮 F3）", () => {
  it("feedingTooltip 逐食物三态：距上次 / 超期标注 / 尚未记录；空明细为空串", () => {
    const foods: FoodTileInfo[] = [
      { food_id: 1, name: "种子", suggested_interval_days: 3, days_since_last: 2, overdue: false },
      { food_id: 3, name: "面包虫", suggested_interval_days: 7, days_since_last: 8, overdue: true },
      { food_id: 2, name: "干虾仁", suggested_interval_days: 7, days_since_last: null, overdue: false },
    ];
    expect(feedingTooltip(foods)).toBe(
      "种子：距上次 2 天\n面包虫：距上次 8 天 · 超期\n干虾仁：尚未记录",
    );
    expect(feedingTooltip([])).toBe("");
  });
});

describe("最近记录摘要", () => {
  it("空记录返回空串（隐藏该行）", () => {
    expect(formatRecent([])).toBe("");
  });

  it("带食物拼「（、连接）」，多行用 · 连接", () => {
    const recent: RecentLog[] = [
      { happened_at: "2026-09-18 08:00:00", action_name: "喂食", food_names: ["干虾仁", "面包虫"] },
      { happened_at: "2026-09-12 09:00:00", action_name: "巢穴保湿", food_names: [] },
    ];
    expect(formatRecent(recent)).toBe(
      "最近：09-18 喂食（干虾仁、面包虫）· 09-12 巢穴保湿",
    );
  });

  it("单条无食物", () => {
    expect(formatRecent([{ happened_at: "2026-09-17 20:00:00", action_name: "喂食", food_names: ["种子"] }])).toBe(
      "最近：09-17 喂食（种子）",
    );
  });
});

describe("时间默认值", () => {
  it("nowLocalDateTime 返回 datetime-local 格式", () => {
    expect(nowLocalDateTime()).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
  });
});
