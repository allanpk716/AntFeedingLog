/**
 * 统计页纯逻辑（票 07）：食物占比归一化（规则 8）、时间范围起算（规则 7 频率
 * 分母的窗口）、每周柱取数、间隔条布局、热力图悬停明细。
 * 视觉基线 mocks/mock-b-stats.html。图表配置组装在 StatsPage.vue。
 */

import type { StatsDayDetail, StatsInterval } from "../types";
import { addDays } from "./dates";

export type StatsRange = "3m" | "6m" | "all";

// ── 食物占比（规则 8）─────────────────────────────────────────────────────

export interface FoodShareSlice {
  food_name: string;
  occurrences: number;
  /** 归一化后的整数百分比；各项合计恰为 100（最大余数法分摊舍入误差）。 */
  pct: number;
}

/**
 * 各食物出现次数 ÷ 总出现次数 → 整数百分比，合计恰为 100%。
 * 一次多选投喂每种各计 1 次由 Rust 端保证；分母在这里归一化。
 * 总次数为 0（空范围）时全部 pct = 0（界面走空态，不参与合计）。
 */
export function normalizeFoodShare(items: { food_name: string; occurrences: number }[]): FoodShareSlice[] {
  const total = items.reduce((s, x) => s + x.occurrences, 0);
  if (total <= 0) {
    return items.map((x) => ({ food_name: x.food_name, occurrences: x.occurrences, pct: 0 }));
  }
  const raw = items.map((x) => (x.occurrences / total) * 100);
  const floors = raw.map((v) => Math.floor(v));
  // 待分发的百分点 = 100 − 各项下取整之和；按小数余数从大到小补 1（同余数先到先得）
  const rest = 100 - floors.reduce((s, f) => s + f, 0);
  const order = raw
    .map((v, i) => ({ i, frac: v - Math.floor(v) }))
    .sort((a, b) => b.frac - a.frac || a.i - b.i);
  const bonus = new Array<number>(items.length).fill(0);
  for (let k = 0; k < rest; k++) {
    bonus[order[k % order.length].i] += 1;
  }
  return items.map((x, i) => ({
    food_name: x.food_name,
    occurrences: x.occurrences,
    pct: floors[i] + bonus[i],
  }));
}

// ── 时间范围（规则 7 频率分母的窗口）─────────────────────────────────────

/**
 * 筛选范围 → 开始日期（YYYY-MM-DD，含首尾）。
 * 近 3/6 个月按自然日窗（90/180 天，含今天）；「全部」= min(各窝最早开始饲养日,
 * 最早记录日 earliestLogDate)——早于饲养日的补录不从统计消失（票 07 停靠①），
 * 且不晚于今天：无窝无记录、或最小值在未来时回退今天（单天范围，不崩不变形）。
 */
export function rangeStartFor(
  range: StatsRange,
  today: string,
  colonies: { start_date: string }[],
  earliestLogDate: string | null = null,
): string {
  if (range === "3m") return addDays(today, -89);
  if (range === "6m") return addDays(today, -179);
  let min: string | null = earliestLogDate;
  for (const c of colonies) {
    if (min === null || c.start_date < min) min = c.start_date;
  }
  if (min === null || min > today) return today;
  return min;
}

// ── 每周柱 ───────────────────────────────────────────────────────────────

/** 取末尾 n 周（不足全给；空数组安全）。周分桶（周一为周首）由 Rust 端完成。 */
export function lastWeeks<T>(weekly: T[], n: number): T[] {
  return weekly.slice(-n);
}

/** "2026-09-07" → "9/7"（柱底标签）。脏日期原样返回，不抛错。 */
export function shortWeek(iso: string): string {
  const m = iso.trim().match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return iso;
  return `${Number(m[2])}/${Number(m[3])}`;
}

// ── 频率口径（规则 7）────────────────────────────────────────────────────

/** 平均每天 X 次 = 范围内总记录数 ÷ 范围自然日天数（不扣冬眠），1 位小数。 */
export function avgPerDay(total: number, rangeDays: number): string {
  if (rangeDays <= 0) return "—";
  return (total / rangeDays).toFixed(1);
}

// ── 间隔条布局（mock-b 的 .itrack/.ibar/.imark）──────────────────────────

export interface IntervalRow extends StatsInterval {
  /** kind 收敛后的建议间隔：仅提醒类透传，登记类置 null（同 suggested_interval_days 的界面语义） */
  suggested: number | null;
  /** 条与刻度共用的量程（最长间隔的 1.15 倍与建议间隔的 1.3 倍取大，保底 1 防除零） */
  scale: number;
  /** 均值条宽（%）；无样本为 null（界面渲染「记录不足」） */
  avgPct: number | null;
  /** 均值文案（1 位小数）；无样本为 null */
  avgLabel: string | null;
  /** 建议刻度竖线位置（%）；仅提醒类有（登记类恒 null，界面标「仅登记」） */
  markPct: number | null;
  /** 右侧文案尾巴："建议 N 天" / "未设建议间隔"（提醒类没设值）/ "仅登记"（按 kind 判定）/
   * "跟随喂食"（follow：撤食等跟随喂食的操作，无建议间隔口径，票 03） */
  tail: string;
}

/** Rust 的间隔行 → 条形布局数据。登记类即使字典里留了间隔值也不画刻度。 */
export function intervalRows(intervals: StatsInterval[]): IntervalRow[] {
  return intervals.map((it) => {
    const suggested = it.kind === "reminding" ? it.suggested_interval_days : null;
    const scale = Math.max((it.max_days ?? 0) * 1.15, (suggested ?? 0) * 1.3, 1);
    const avgPct = it.avg_days === null ? null : Math.min((it.avg_days / scale) * 100, 100);
    const markPct = suggested === null ? null : Math.min((suggested / scale) * 100, 100);
    return {
      ...it,
      suggested,
      scale,
      avgPct,
      avgLabel: it.avg_days === null ? null : it.avg_days.toFixed(1),
      markPct,
      // 按 kind 判定（票 07 停靠②）：提醒类但没设建议间隔时不误标「仅登记」；
      // follow（跟随喂食，票 03）走兜底文案——它没有间隔口径，也不算「未设建议」
      tail:
        it.kind === "log_only"
          ? "仅登记"
          : it.kind === "reminding"
            ? suggested === null
              ? "未设建议间隔"
              : `建议 ${suggested} 天`
            : "跟随喂食",
    };
  });
}

// ── 热力图悬停明细（验收 3：明细含喂食的食物）────────────────────────────

/** 按日期建索引，供 tooltip O(1) 查询。 */
export function buildDetailMap(detail: StatsDayDetail[]): Map<string, StatsDayDetail["entries"]> {
  return new Map(detail.map((d) => [d.date, d.entries]));
}

/** 悬停文案：`2026-09-10 · 2 次：喂食（种子、干虾仁）、巢穴保湿`；无记录 → `… · 无记录`。 */
export function dayTooltip(date: string, map: Map<string, StatsDayDetail["entries"]>): string {
  const entries = map.get(date);
  if (!entries || entries.length === 0) {
    return `${date} · 无记录`;
  }
  const items = entries.map((e) =>
    e.food_names.length > 0 ? `${e.action_name}（${e.food_names.join("、")}）` : e.action_name,
  );
  return `${date} · ${entries.length} 次：${items.join("、")}`;
}
