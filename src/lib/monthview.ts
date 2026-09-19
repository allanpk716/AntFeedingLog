/**
 * 打卡日历的记录标记与重复提醒纯逻辑（交互第三轮 #3/#8）。
 * 数据源 colony_month_records（一次一窝一月）；口径与 mocks/mock-c-calendar.html 一致：
 * 橙点=当天已有「正在记录的这项操作」，灰点=当天有其它操作，悬停 tip 列明细。
 */
import type { MonthDayRecords } from "../types";

const iso = (year: number, month: number, day: number) =>
  `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;

/** rows → 日历标记表：按日聚合，tip = "操作A×n · 操作B×m"（actionNames 缺名时跳过该行） */
export function buildMarkers(
  rows: MonthDayRecords[],
  currentActionId: number,
  actionNames: Map<number, string>,
): Record<string, { current: boolean; tip: string }> {
  const byDay = new Map<number, MonthDayRecords[]>();
  for (const r of rows) {
    const list = byDay.get(r.day) ?? [];
    list.push(r);
    byDay.set(r.day, list);
  }
  const out: Record<string, { current: boolean; tip: string }> = {};
  for (const [day, list] of byDay) {
    const parts: string[] = [];
    let current = false;
    // 明细按 rows 到达序输出（Rust 端 ORDER BY day, action_id 已定序；复审 #4：不做二次排序，与测试口径一致）
    for (const r of list) {
      const name = actionNames.get(r.action_id);
      if (name === undefined) continue;
      parts.push(`${name}×${r.count}`);
      if (r.action_id === currentActionId) current = true;
    }
    if (parts.length > 0) out[dayKey(list[0]!, day)] = { current, tip: parts.join(" · ") };
  }
  return out;
}

/** rows 里挑出 ISO 日期的 key 用的年份月份（rows 行自带 last_time 可取年月） */
function dayKey(sample: MonthDayRecords, day: number): string {
  const y = +sample.last_time.slice(0, 4);
  const m = +sample.last_time.slice(5, 7);
  return iso(y, m, day);
}

/** 选中日期已有同操作记录 → {count, 最近 HH:MM}；否则 null（跨月/无记录都 null） */
export function duplicateInfo(
  rows: MonthDayRecords[],
  year: number,
  month: number,
  dateIso: string,
  actionId: number,
): { count: number; lastTime: string } | null {
  if (!dateIso.startsWith(`${year}-${String(month).padStart(2, "0")}-`)) return null;
  const day = +dateIso.slice(8, 10);
  const hit = rows.find((r) => r.day === day && r.action_id === actionId);
  return hit === undefined ? null : { count: hit.count, lastTime: hit.last_time.slice(11, 16) };
}

/** 黄条文案：今天 / 其它日期两形态；无重复返回 ""（弹窗据此隐藏黄条） */
export function dupWarningText(
  dup: { count: number; lastTime: string } | null,
  dateIso: string,
  todayIsoStr: string,
  actionName: string,
): string {
  if (dup === null) return "";
  const when = dateIso === todayIsoStr
    ? "今天"
    : `${+dateIso.slice(5, 7)}月${+dateIso.slice(8, 10)}日`;
  const recent = dup.count > 1 ? `（最近 ${dup.lastTime}）` : `（${dup.lastTime}）`;
  return `${when}已有 ${dup.count} 条${actionName}记录${recent}，请确认不是重复操作。`;
}
