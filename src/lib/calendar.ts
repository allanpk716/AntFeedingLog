/**
 * 自绘日历纯逻辑（交互第三轮 #3）：月格构建 + datetime-local 形态字符串的时间步进。
 * 组件只管渲染与弹层，格子/进位规则全部在这里，口径与 mocks/mock-c-calendar.html 一致。
 */

/** 周一起排的月格：前置 null 填充（周列头由组件渲染），当月每天给 ISO 日期 */
export function monthGrid(year: number, month: number): { iso: string | null }[] {
  if (month < 1 || month > 12) {
    throw new Error(`月份应在 1–12：${year}-${month}`);
  }
  const lead = (new Date(year, month - 1, 1).getDay() + 6) % 7; // 周一=0
  const days = new Date(year, month, 0).getDate();
  const cells: { iso: string | null }[] = Array.from({ length: lead }, () => ({ iso: null }));
  for (let d = 1; d <= days; d++) {
    cells.push({ iso: `${year}-${String(month).padStart(2, "0")}-${String(d).padStart(2, "0")}` });
  }
  return cells;
}

/** ISO 日期 → 一/二/…/日（标题展示用） */
export function weekdayShort(iso: string): string {
  return "一二三四五六日"[(new Date(`${iso}T00:00`).getDay() + 6) % 7];
}

/** "YYYY-MM-DDTHH:mm" → 总分钟数；解析失败 NaN */
function toMinutes(value: string): number {
  const m = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/);
  if (!m) return Number.NaN;
  const [, y, mo, d, h, mi] = m;
  return Date.UTC(+y, +mo - 1, +d, +h, +mi) / 60000;
}

function fromMinutes(total: number): string {
  const dt = new Date(total * 60000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${dt.getUTCFullYear()}-${p(dt.getUTCMonth() + 1)}-${p(dt.getUTCDate())}T${p(dt.getUTCHours())}:${p(dt.getUTCMinutes())}`;
}

/** 整体平移分钟（现在/±10分快捷键）；跨日安全；脏值原样返回 */
export function shiftMinutes(value: string, delta: number): string {
  const t = toMinutes(value);
  return Number.isNaN(t) ? value : fromMinutes(t + delta);
}

/** 单字段步进（时/分 ‹›）——复审定稿语义：分钟算术进位（59+1→下一小时 00）、
 * 小时 23↔0 回绕；步进不跨日（跨日走 ±10分/现在）；脏值原样返回。 */
export function stepTimeField(value: string, field: "h" | "m", delta: number): string {
  const m = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/);
  if (!m) return value;
  const [, y, mo, d, h, mi] = m;
  const total =
    field === "h"
      ? ((((+h + delta) % 24) + 24) % 24) * 60 + +mi // 小时回绕，分钟不动
      : (+h * 60 + +mi + delta + 1440) % 1440; // 分钟算术进位可跨小时，日期不动
  const p = (n: number) => String(n).padStart(2, "0");
  return `${y}-${mo}-${d}T${p(Math.floor(total / 60))}:${p(total % 60)}`;
}
