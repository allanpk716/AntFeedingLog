import { describe, expect, it } from "vitest";
import type { ColonyAction, FoodTileInfo, RecentLog } from "../types";
import {
  actionTile,
  feedingTooltip,
  formatRecent,
  isFeeding,
  isRetrieval,
  nowLocalDateTime,
  retrievalStateOf,
  retrievalTile,
  type RetrievalState,
} from "./care";

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

describe("每窝周期三形态（票 03）", () => {
  /** 该窝设了每窝周期的操作（默认有效周期 7 天来自每窝；partial 可覆盖）。
   *  判「设了」的唯一可信源是 interval_from_colony（票 02 评审要领），构造器即如此摆字段。 */
  function colonyIntervalAction(
    partial: Partial<ColonyAction> & { action_id: number },
  ): ColonyAction {
    return action({
      interval_from_colony: true,
      effective_interval_days: 7,
      ...partial,
    });
  }

  function food(id: number, name: string, overdue: boolean): FoodTileInfo {
    return {
      food_id: id,
      name,
      suggested_interval_days: 7,
      days_since_last: overdue ? 8 : 1,
      overdue,
    };
  }

  it("形态①：设了周期未超 →「距上次 N 天 / 周期 M 天」（M=该窝有效周期）", () => {
    expect(
      actionTile(colonyIntervalAction({ action_id: 1, days_since_last: 2 }), false),
    ).toEqual({ tone: "ok", text: "距上次 2 天 / 周期 7 天" });
    // 当天刚记录也带周期上下文
    expect(
      actionTile(colonyIntervalAction({ action_id: 1, days_since_last: 0 }), false),
    ).toEqual({ tone: "ok", text: "今天 · 已记录 / 周期 7 天" });
  });

  it("形态①喂食类同样生效：每窝周期统一层新鲜、食物也新鲜 → 距上次 / 周期", () => {
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 1,
          is_feeding: true,
          days_since_last: 5,
          overdue: false,
          foods: [food(3, "面包虫", false)],
        }),
        false,
      ),
    ).toEqual({ tone: "ok", text: "距上次 5 天 / 周期 7 天" });
  });

  it("形态②：⚠ 超期 X 天与判定同源——X = days − 每窝有效周期，不吃操作层旧值", () => {
    // 每窝周期 3 天；操作层残留建议间隔 100：超期 1 天（4−3），绝非按 100 算
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 1,
          effective_interval_days: 3,
          suggested_interval_days: 100,
          days_since_last: 4,
          overdue: true,
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 1 天" });
  });

  it("形态②边界：严格大于（days==周期不红走形态①，+1 天红且超 1）", () => {
    expect(
      actionTile(
        colonyIntervalAction({ action_id: 1, effective_interval_days: 4, days_since_last: 4 }),
        false,
      ),
    ).toEqual({ tone: "ok", text: "距上次 4 天 / 周期 4 天" });
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 1,
          effective_interval_days: 4,
          days_since_last: 5,
          overdue: true,
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 1 天" });
  });

  it("形态③：未设周期与现状逐字节一致——残留 effective 值也不显周期、口径照旧", () => {
    // 登记类切性质不清空间隔值：suggested/effective 残留 Some 但 interval_from_colony 缺省
    // —— 只显示距上次、永不红（票 02 评审要领 1：不拿 effective != null 当「设了」）
    expect(
      actionTile(
        action({
          action_id: 2,
          kind: "log_only",
          suggested_interval_days: 3,
          effective_interval_days: 3,
          days_since_last: 100,
        }),
        false,
      ),
    ).toEqual({ tone: "reg", text: "距上次 100 天" });
    // 提醒类未设每窝周期：超期仍按建议间隔算、不追加「周期 M 天」
    expect(
      actionTile(
        action({
          action_id: 1,
          suggested_interval_days: 3,
          effective_interval_days: 3,
          days_since_last: 4,
          overdue: true,
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 1 天" });
    expect(
      actionTile(
        action({
          action_id: 1,
          suggested_interval_days: 3,
          effective_interval_days: 3,
          days_since_last: 2,
        }),
        false,
      ),
    ).toEqual({ tone: "ok", text: "距上次 2 天" });
  });

  it("设了即提醒：登记类设了每窝周期，未超中性灰带周期、超期照红", () => {
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 2,
          kind: "log_only",
          effective_interval_days: 14,
          days_since_last: 3,
        }),
        false,
      ),
    ).toEqual({ tone: "reg", text: "距上次 3 天 / 周期 14 天" });
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 2,
          kind: "log_only",
          effective_interval_days: 14,
          days_since_last: 20,
          overdue: true,
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 6 天" });
  });

  it("食物行照旧独立标超期：每窝周期统一层新鲜时红由食物层顶，口径不因每窝周期变", () => {
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 1,
          is_feeding: true,
          days_since_last: 2,
          overdue: true,
          foods: [food(3, "面包虫", true)],
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 该喂面包虫了" });
    // 每窝周期统一层自己超了：维持「超期 N 天」原句式（措辞优先于食物层，与现状同构）
    expect(
      actionTile(
        colonyIntervalAction({
          action_id: 1,
          is_feeding: true,
          effective_interval_days: 3,
          days_since_last: 8,
          overdue: true,
          foods: [food(3, "面包虫", true)],
        }),
        false,
      ),
    ).toEqual({ tone: "bad", text: "⚠ 超期 5 天" });
  });

  it("边界照旧：从未记录不催不显周期；冬眠静音不加周期、永不红", () => {
    expect(
      actionTile(colonyIntervalAction({ action_id: 1, days_since_last: null }), false),
    ).toEqual({ tone: "none", text: "尚未记录" });
    expect(
      actionTile(colonyIntervalAction({ action_id: 1, days_since_last: 9, overdue: true }), true),
    ).toEqual({ tone: "mute", text: "距上次 9 天（静音）" });
    expect(
      actionTile(colonyIntervalAction({ action_id: 1, days_since_last: 2 }), true),
    ).toEqual({ tone: "mute", text: "距上次 2 天（静音）" });
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

describe("撤食块展示态（票 02）", () => {
  /** follow 性质的撤食块；state 缺省 = 模拟旧载荷（无 retrieval_state 字段） */
  function retrievalAction(
    state?: RetrievalState,
  ): ColonyAction & { retrieval_state?: RetrievalState } {
    const base = action({
      action_id: 9,
      name: "撤食",
      kind: "follow",
      suggested_interval_days: null,
      days_since_last: 1,
    });
    return state === undefined ? base : { ...base, retrieval_state: state };
  }

  it("follow 性质判定与名字无关", () => {
    expect(isRetrieval(retrievalAction("none"))).toBe(true);
    expect(isRetrieval(retrievalAction())).toBe(true);
    // 名字叫「撤食」但性质不是 follow → 不是撤食块
    expect(isRetrieval(action({ action_id: 5, name: "撤食", kind: "reminding" }))).toBe(false);
    expect(isRetrieval(action({ action_id: 2, kind: "log_only" }))).toBe(false);
  });

  it("三态映射：无待撤置灰 / 待撤未到期正常可点 / 逾期红", () => {
    expect(retrievalTile(retrievalAction("none"))).toEqual({ tone: "none", text: "无待撤" });
    expect(retrievalTile(retrievalAction("pending"))).toEqual({ tone: "ok", text: "待撤食" });
    expect(retrievalTile(retrievalAction("overdue"))).toEqual({ tone: "bad", text: "⚠ 该撤食了" });
  });

  it("无倒计时文字：pending/overdue 文案都不含小时/天/countdown 灰字", () => {
    for (const state of ["none", "pending", "overdue"] as const) {
      const view = retrievalTile(retrievalAction(state));
      expect(view.text).not.toMatch(/小时|天/);
    }
  });

  it("缺 retrieval_state 字段（旧载荷/非 follow）按 none 兜底", () => {
    expect(retrievalStateOf(retrievalAction())).toBe("none");
    expect(retrievalTile(action({ action_id: 9, kind: "log_only" }))).toEqual({
      tone: "none",
      text: "无待撤",
    });
  });
});
