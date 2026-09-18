/**
 * 日期纯函数。饲养天数与 Rust 端（src-tauri/src/colony.rs::days_raised）同口径：
 * 今天 − 开始饲养日期的自然日天数（含冬眠）。Rust 返回的 days_raised 是展示权威值，
 * 这里的实现供前端需要自行换算时使用（乐观更新、兜底重算等）。
 * 全部走 UTC 换算，不受本地时区/夏令时影响。
 */

const ISO_RE = /^(\d{4})-(\d{2})-(\d{2})$/;
const MS_PER_DAY = 86_400_000;

/** 解析 YYYY-MM-DD 为 UTC 时间戳；格式错或日期不存在（如 02-30）抛错。 */
function parseIsoDateUtc(iso: string): number {
  const match = iso.trim().match(ISO_RE);
  if (!match) {
    throw new Error(`日期格式应为 YYYY-MM-DD：${iso}`);
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const utc = Date.UTC(year, month - 1, day);
  const date = new Date(utc);
  // 回读比对：2 月 30 日这类会被 Date 进位，进位即说明日期不存在
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    throw new Error(`日期不存在：${iso}`);
  }
  return utc;
}

export function isValidIsoDate(iso: string): boolean {
  try {
    parseIsoDateUtc(iso);
    return true;
  } catch {
    return false;
  }
}

/** 饲养天数：today − start 的自然日数；同一天 0；开始日在未来为负。 */
export function daysRaised(startIso: string, todayIso: string): number {
  const start = parseIsoDateUtc(startIso);
  const today = parseIsoDateUtc(todayIso);
  return Math.round((today - start) / MS_PER_DAY);
}

/** to − from 的自然日数（半开口径不含端点进位，同 daysRaised：同一天 0，to 更早为负）。 */
export function daysBetween(fromIso: string, toIso: string): number {
  return Math.round((parseIsoDateUtc(toIso) - parseIsoDateUtc(fromIso)) / MS_PER_DAY);
}

/** iso + days 天的 YYYY-MM-DD（自动进位月/年，days 可为负）。 */
export function addDays(iso: string, days: number): string {
  const date = new Date(parseIsoDateUtc(iso) + days * MS_PER_DAY);
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
}

/** 本机今天的 YYYY-MM-DD（表单默认值用）。 */
export function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

const WEEKDAY_LABELS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const;

/** 顶栏日期标签（票 09 停靠 F，对齐 mock-a-light 的 .today）：本机今天 "YYYY-MM-DD 周X"。 */
export function todayLabel(now: Date = new Date()): string {
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${
    WEEKDAY_LABELS[now.getDay()]
  }`;
}
