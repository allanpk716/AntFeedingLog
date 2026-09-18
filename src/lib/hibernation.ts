/**
 * 冬眠横幅与默认值的纯逻辑（票 05）。权威数据（开放段、约束校验、重叠扣减）
 * 在 Rust（src-tauri/src/hibernation.rs），这里只做展示文案拼装：
 * 横幅 = 入眠日 / 预计出眠 / 剩余天数（视觉文案对齐 mock-a-light），
 * 预计出眠前 aheadDays 天内（默认 7，出眠当天与已过仍算）显示「临近出眠」角标。
 * 通知调度本身属票 06，此处角标纯展示。
 */

import type { HibernationPreview } from "../types";
import { addDays, daysBetween } from "./dates";

/** 开始冬眠时预计出眠日的默认预填：开始日 + 120 天（可改）。 */
export function defaultExpectedEnd(startIso: string): string {
  return addDays(startIso, 120);
}

export interface HibernationBannerView {
  /** 横幅主文案：「❄ 冬眠中 · 08-20 入眠 · 预计出眠 09-25（还有 7 天）」 */
  line: string;
  /** 今天 −> 预计出眠日的自然日数：正=还有 N 天，0=当天，负=已过 N 天 */
  remainingDays: number;
  /** 预计出眠前 aheadDays 天内（含当天与已过）→「临近出眠」角标 */
  nearWake: boolean;
}

/** 开放段 → 横幅展示数据。today 为本机今天（YYYY-MM-DD）。 */
export function hibernationBanner(
  seg: HibernationPreview,
  today: string,
  aheadDays = 7,
): HibernationBannerView {
  const remaining = daysBetween(today, seg.expected_end_date);
  const remainText =
    remaining > 0
      ? `还有 ${remaining} 天`
      : remaining === 0
        ? "今天预计出眠"
        : `预计日已过 ${-remaining} 天`;
  return {
    line:
      `❄ 冬眠中 · ${seg.start_date.slice(5, 10)} 入眠 · ` +
      `预计出眠 ${seg.expected_end_date.slice(5, 10)}（${remainText}）`,
    remainingDays: remaining,
    nearWake: remaining <= aheadDays,
  };
}
