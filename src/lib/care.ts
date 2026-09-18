/**
 * 记账与操作块展示的纯逻辑（票 03）。「距上次/超期」的权威计算在 Rust
 * （src-tauri/src/care.rs），这里只做展示态判定与文案拼装，口径与 mock-a-light 一致：
 * 提醒类三态（今天·已记录 / 距上次 X 天 / ⚠ 超期 N 天），登记类中性灰 +「仅登记」角标，
 * 冬眠整卡静音（票 05 接管横幅）。
 */

import type { ColonyAction, RecentLog } from "../types";

/** 操作块展示态：ok=绿 / bad=红（超期）/ reg=中性灰（登记类）/ none=尚未记录 / mute=冬眠静音 */
export type TileTone = "ok" | "bad" | "reg" | "none" | "mute";

export interface TileView {
  tone: TileTone;
  text: string;
}

/** 操作块 → 展示态。hibernating 时整卡静音：永不红，文案加「（静音）」。 */
export function actionTile(a: ColonyAction, hibernating: boolean): TileView {
  const days = a.days_since_last;
  const baseText = days === null ? "尚未记录" : days === 0 ? "今天 · 已记录" : `距上次 ${days} 天`;
  if (hibernating) {
    return { tone: "mute", text: `${baseText}（静音）` };
  }
  if (a.kind === "log_only") {
    return { tone: "reg", text: baseText };
  }
  if (a.overdue && days !== null) {
    const overDays = days - (a.suggested_interval_days ?? 0);
    return { tone: "bad", text: `⚠ 超期 ${overDays} 天` };
  }
  if (days === null) {
    return { tone: "none", text: baseText };
  }
  return { tone: "ok", text: baseText };
}

/** 是否喂食类操作（点它弹食物多选，其余一点即记）。按 schema 的 is_feeding 标记位，与名字无关。 */
export function isFeeding(a: ColonyAction): boolean {
  return a.is_feeding;
}

/**
 * 最近记录摘要行：「最近：09-17 喂食（种子、干虾仁）· 09-12 巢穴保湿」。
 * 无记录返回空串（调用方隐藏该行）。
 */
export function formatRecent(recent: RecentLog[]): string {
  if (recent.length === 0) {
    return "";
  }
  const parts = recent.map((r) => {
    const day = r.happened_at.slice(5, 10); // "2026-09-17 …" → "09-17"
    const foods = r.food_names.length > 0 ? `（${r.food_names.join("、")}）` : "";
    return `${day} ${r.action_name}${foods}`;
  });
  return `最近：${parts.join("· ")}`;
}

/** 本机当前时间的 datetime-local 值：「YYYY-MM-DDTHH:MM」（喂食弹窗时间默认值）。 */
export function nowLocalDateTime(): string {
  const n = new Date();
  const pad = (v: number) => String(v).padStart(2, "0");
  return (
    `${n.getFullYear()}-${pad(n.getMonth() + 1)}-${pad(n.getDate())}` +
    `T${pad(n.getHours())}:${pad(n.getMinutes())}`
  );
}
