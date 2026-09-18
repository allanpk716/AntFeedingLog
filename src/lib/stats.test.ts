import { describe, expect, it } from "vitest";
import { addDays } from "./dates";
import {
  avgPerDay,
  buildDetailMap,
  dayTooltip,
  intervalRows,
  lastWeeks,
  normalizeFoodShare,
  rangeStartFor,
  shortWeek,
} from "./stats";
import type { StatsInterval, StatsPayload } from "../types";

/** 与 Rust stats.rs 验收用例同口径的样例 payload */
function makePayload(overrides: Partial<StatsPayload> = {}): StatsPayload {
  return {
    range_start: "2026-06-22",
    range_end: "2026-09-18",
    range_days: 89,
    daily: [
      { date: "2026-09-10", count: 2 },
      { date: "2026-09-12", count: 1 },
    ],
    daily_detail: [
      {
        date: "2026-09-10",
        entries: [
          { action_name: "喂食", food_names: ["种子", "干虾仁"] },
          { action_name: "巢穴保湿", food_names: [] },
        ],
      },
    ],
    food_share: [
      { food_name: "种子", occurrences: 2 },
      { food_name: "干虾仁", occurrences: 1 },
    ],
    weekly: [{ week_start: "2026-09-14", count: 3 }],
    intervals: [],
    ...overrides,
  };
}

describe("食物占比归一化（规则 8，验收 1：各分项合计 = 100%）", () => {
  it("均分场景：[1,1,1] → 34/33/33，合计恰为 100", () => {
    const slices = normalizeFoodShare([
      { food_name: "种子", occurrences: 1 },
      { food_name: "干虾仁", occurrences: 1 },
      { food_name: "面包虫", occurrences: 1 },
    ]);
    expect(slices.map((s) => s.pct)).toEqual([34, 33, 33]);
    expect(slices.reduce((s, x) => s + x.pct, 0)).toBe(100);
  });

  it("非整除场景用最大余数法补齐：[3,3,2] → 38/37/25，合计 100", () => {
    const slices = normalizeFoodShare([
      { food_name: "种子", occurrences: 3 },
      { food_name: "干虾仁", occurrences: 3 },
      { food_name: "面包虫", occurrences: 2 },
    ]);
    expect(slices.map((s) => s.pct)).toEqual([38, 37, 25]);
    expect(slices.reduce((s, x) => s + x.pct, 0)).toBe(100);
  });

  it("单食物 = 100%；次数原样保留；不改动入参", () => {
    const input = [{ food_name: "种子", occurrences: 5 }];
    const slices = normalizeFoodShare(input);
    expect(slices[0].pct).toBe(100);
    expect(slices[0].occurrences).toBe(5);
    expect(input).toEqual([{ food_name: "种子", occurrences: 5 }]);
  });

  it("总次数为 0（空范围）时全部 0，不产生 NaN", () => {
    const slices = normalizeFoodShare([{ food_name: "种子", occurrences: 0 }]);
    expect(slices[0].pct).toBe(0);
  });
});

describe("时间范围起算（规则 7 频率分母的窗口）", () => {
  const today = "2026-09-18";

  it("近 3 个月 = 含今天共 90 天的窗口起点", () => {
    expect(rangeStartFor("3m", today, [])).toBe(addDays(today, -89));
  });

  it("近 6 个月 = 含今天共 180 天的窗口起点", () => {
    expect(rangeStartFor("6m", today, [])).toBe(addDays(today, -179));
  });

  it("全部 = 各窝最早开始饲养日期（无更早记录时）", () => {
    const colonies = [{ start_date: "2026-03-01" }, { start_date: "2025-12-20" }];
    expect(rangeStartFor("all", today, colonies)).toBe("2025-12-20");
    // 记录晚于饲养日时不改变下界
    expect(rangeStartFor("all", today, colonies, "2026-01-05")).toBe("2025-12-20");
  });

  it("全部：早于饲养日的补录把下界压到最早记录日（票 07 停靠①）", () => {
    const colonies = [{ start_date: "2026-03-01" }];
    expect(rangeStartFor("all", today, colonies, "2025-12-01")).toBe("2025-12-01");
  });

  it("全部：最早记录日参与比较，min 落在未来才回退今天（不变形）", () => {
    expect(rangeStartFor("all", today, [{ start_date: "2027-01-01" }], "2027-02-01")).toBe(today);
    expect(rangeStartFor("all", today, [{ start_date: "2026-03-01" }], "2027-01-01")).toBe("2026-03-01");
  });

  it("全部但无窝 / 最早开始日在未来 → 今天兜底（单天范围，不崩）", () => {
    expect(rangeStartFor("all", today, [])).toBe(today);
    expect(rangeStartFor("all", today, [{ start_date: "2027-01-01" }])).toBe(today);
  });
});

describe("每周柱取数与标签", () => {
  it("lastWeeks 取末尾 12 周，不足全给，空数组安全", () => {
    const weeks = Array.from({ length: 14 }, (_, i) => ({ week_start: `2026-w${i}`, count: i }));
    expect(lastWeeks(weeks, 12)).toHaveLength(12);
    expect(lastWeeks(weeks, 12)[11].count).toBe(13);
    expect(lastWeeks(weeks.slice(0, 3), 12)).toHaveLength(3);
    expect(lastWeeks([], 12)).toEqual([]);
  });

  it("shortWeek 压成 M/D 展示格式", () => {
    expect(shortWeek("2026-09-07")).toBe("9/7");
    expect(shortWeek("2025-12-28")).toBe("12/28");
  });
});

describe("频率口径（规则 7：分母 = 范围自然日天数）", () => {
  it("平均每天 = 总数 ÷ 天数，保留 1 位小数", () => {
    expect(avgPerDay(10, 7)).toBe("1.4");
    expect(avgPerDay(0, 90)).toBe("0.0");
    expect(avgPerDay(3, 89)).toBe("0.0");
  });

  it("天数为 0（不应发生）时回退占位，不产生 NaN", () => {
    expect(avgPerDay(5, 0)).toBe("—");
  });
});

describe("间隔条布局（照 mock-b .itrack/.ibar/.imark）", () => {
  const feed: StatsInterval = {
    action_id: 1,
    name: "喂食",
    kind: "reminding",
    suggested_interval_days: 3,
    sample_count: 2,
    avg_days: 6,
    min_days: 5,
    max_days: 7,
  };

  it("提醒类：均值条与建议刻度同尺度换算成百分比，文案带建议 N 天", () => {
    const [row] = intervalRows([feed]);
    // scale = max(7*1.15, 3*1.3, 1) = 8.05
    expect(row.scale).toBeCloseTo(8.05);
    expect(row.avgPct).toBeCloseTo((6 / 8.05) * 100);
    expect(row.markPct).toBeCloseTo((3 / 8.05) * 100);
    expect(row.tail).toBe("建议 3 天");
    expect(row.suggested).toBe(3);
  });

  it("登记类：不带建议刻度（suggested 置空），文案标「仅登记」", () => {
    const water: StatsInterval = {
      action_id: 2,
      name: "活动区换水",
      kind: "log_only",
      suggested_interval_days: 99,
      sample_count: 1,
      avg_days: 10,
      min_days: 10,
      max_days: 10,
    };
    const [row] = intervalRows([water]);
    expect(row.suggested).toBeNull();
    expect(row.markPct).toBeNull();
    expect(row.tail).toBe("仅登记");
  });

  it("提醒类但未设建议间隔：文案不误标「仅登记」（票 07 停靠②，按 kind 判定）", () => {
    const noSuggestion: StatsInterval = {
      action_id: 3,
      name: "糖水",
      kind: "reminding",
      suggested_interval_days: null,
      sample_count: 2,
      avg_days: 6,
      min_days: 5,
      max_days: 7,
    };
    const [row] = intervalRows([noSuggestion]);
    expect(row.suggested).toBeNull();
    expect(row.markPct).toBeNull();
    expect(row.tail).toBe("未设建议间隔");
    expect(row.tail).not.toContain("仅登记");
  });

  it("无样本（记录不足）：avgPct 为 null，不产生 NaN", () => {
    const none: StatsInterval = { ...feed, sample_count: 0, avg_days: null, min_days: null, max_days: null };
    const [row] = intervalRows([none]);
    expect(row.avgPct).toBeNull();
    expect(row.markPct).not.toBeNull();
  });

  it("全零间隔（同天连记）也给出可用刻度，不除零", () => {
    const zero: StatsInterval = { ...feed, avg_days: 0, min_days: 0, max_days: 0 };
    const [row] = intervalRows([zero]);
    expect(row.avgPct).toBe(0);
    expect(Number.isFinite(row.markPct)).toBe(true);
  });
});

describe("热力图悬停明细（验收 3：明细含喂食的食物）", () => {
  const payload = makePayload();

  it("buildDetailMap 按日期建索引；dayTooltip 拼日期 + 次数 + 明细（食物括注）", () => {
    const map = buildDetailMap(payload.daily_detail);
    expect(dayTooltip("2026-09-10", map)).toBe("2026-09-10 · 2 次：喂食（种子、干虾仁）、巢穴保湿");
    expect(dayTooltip("2026-09-12", map)).toBe("2026-09-12 · 无记录");
  });
});
