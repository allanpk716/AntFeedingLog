/**
 * 巢况登记展示纯函数（webui-checkin 票 02）。权威计算（基线/距上次天数）在
 * Rust（src-tauri/src/nest_checkin.rs），这里只拼展示文案，口径与既有 care.ts
 * 的「距上次 X 天」句式一致。
 */

import type { CheckinDigest, NestCheckin } from "../types";

/** 蚁后/工蚁数文案：都未数为空串；单项只出该项；两项用「 · 」连接。 */
export function countsText(queenCount: number | null, workerCount: number | null): string {
  const parts: string[] = [];
  if (queenCount !== null) parts.push(`蚁后 ${queenCount}`);
  if (workerCount !== null) parts.push(`工蚁 ${workerCount}`);
  return parts.join(" · ");
}

/** 距上次登记：当天 0 =「今天登记」，其余「距上次登记 N 天」。 */
export function daysSinceText(days: number): string {
  return days === 0 ? "今天登记" : `距上次登记 ${days} 天`;
}

/**
 * 窝卡片巢况行：「巢况：蚁后 2 · 工蚁 3000 · 距上次登记 3 天」（换巢插在天数前）。
 * 从未登记返回空串（调用方隐藏该行）。
 */
export function checkinCardLine(digest: CheckinDigest): string {
  const latest = digest.latest;
  if (latest === null) {
    return "";
  }
  const parts: string[] = [];
  const counts = countsText(latest.queen_count, latest.worker_count);
  if (counts !== "") parts.push(counts);
  if (latest.moved_nest) parts.push("换巢");
  if (digest.days_since_last !== null) parts.push(daysSinceText(digest.days_since_last));
  return `巢况：${parts.join(" · ")}`;
}

/** 时间线单条主文案：数/换巢/备注按序用「 · 」连接（日期由调用方单独渲染）。 */
export function checkinEntryLine(c: NestCheckin): string {
  const parts: string[] = [];
  const counts = countsText(c.queen_count, c.worker_count);
  if (counts !== "") parts.push(counts);
  if (c.moved_nest) parts.push("换巢");
  if (c.note !== "") parts.push(`备注：${c.note}`);
  return parts.join(" · ");
}
