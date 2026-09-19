/**
 * 记账与操作块展示的纯逻辑（票 03）。「距上次/超期」的权威计算在 Rust
 * （src-tauri/src/care.rs），这里只做展示态判定与文案拼装，口径与 mock-a-light 一致：
 * 提醒类三态（今天·已记录 / 距上次 X 天 / ⚠ 超期 N 天），登记类中性灰 +「仅登记」角标，
 * 冬眠整卡静音（票 05 接管横幅）；每窝周期三形态见 actionTile（票 03，数据来自票 02 的
 * ActionTile.effective_interval_days / interval_from_colony）。
 */

import type { ColonyAction, FoodTileInfo, RecentLog } from "../types";

/** 操作块展示态：ok=绿 / bad=红（超期）/ reg=中性灰（登记类）/ none=尚未记录 / mute=冬眠静音 */
export type TileTone = "ok" | "bad" | "reg" | "none" | "mute";

export interface TileView {
  tone: TileTone;
  text: string;
}

/** 操作块 → 展示态。hibernating 时整卡静音：永不红，文案加「（静音）」。
 *  每窝周期三形态（票 03）：设了每窝周期未超 =「距上次 N 天 / 周期 M 天」（M=有效周期）、
 *  超期 =「⚠ 超期 X 天」（X = days − 有效周期，与 Rust is_overdue_effective 同源）、
 *  未设 = 与原行为逐字节一致。 */
export function actionTile(a: ColonyAction, hibernating: boolean): TileView {
  const days = a.days_since_last;
  const baseText = days === null ? "尚未记录" : days === 0 ? "今天 · 已记录" : `距上次 ${days} 天`;
  if (hibernating) {
    return { tone: "mute", text: `${baseText}（静音）` };
  }
  // 判「该窝设了每窝周期」只认 interval_from_colony（票 02 评审要领：
  // effective_interval_days 可能残留操作层旧值，!= null 会把没设的窝误判成设了）。
  const fromColony = a.interval_from_colony === true;
  // 超期天数与判定同源（spec 钉死 F2）：设了减每窝有效周期，未设维持建议间隔原句式。
  const effectiveInterval =
    fromColony ? a.effective_interval_days ?? null : a.suggested_interval_days ?? null;
  if (a.overdue && days !== null) {
    if (effectiveInterval !== null && days > effectiveInterval) {
      // 操作层自己超期（含每窝周期取代）：维持「超期 N 天」原句式（措辞优先于食物层）
      return { tone: "bad", text: `⚠ 超期 ${days - effectiveInterval} 天` };
    }
    // 红是食物层顶的（统一层还新鲜，拿它算超期天数会出负数）：报该喂哪些食物
    const names = a.foods
      .filter((f) => f.overdue)
      .map((f) => f.name)
      .join("、");
    return { tone: "bad", text: `⚠ 该喂${names}了` };
  }
  // 形态①：设了每窝周期且未超——距上次（或今天·已记录）+「/ 周期 M 天」；
  // 登记类设了也照显（设了即提醒，Rust is_overdue_effective 同构），tone 仍按性质走。
  const periodSuffix =
    fromColony && effectiveInterval !== null && days !== null
      ? ` / 周期 ${effectiveInterval} 天`
      : "";
  if (a.kind === "log_only") {
    return { tone: "reg", text: baseText + periodSuffix };
  }
  if (days === null) {
    return { tone: "none", text: baseText };
  }
  return { tone: "ok", text: baseText + periodSuffix };
}

/** 是否喂食类操作（点它弹食物多选，其余一点即记）。按 schema 的 is_feeding 标记位，与名字无关。 */
export function isFeeding(a: ColonyAction): boolean {
  return a.is_feeding;
}

/** 撤食块三态（票 02，Rust ActionTile.retrieval_state）：
 *  none=无待撤 / pending=待撤未到期 / overdue=已超到期时刻 */
export type RetrievalState = "none" | "pending" | "overdue";

/** 撤食块（follow 性质）判定：与名字无关（follow 为撤食预置专属性质，票 01）。 */
export function isRetrieval(a: ColonyAction): boolean {
  return a.kind === "follow";
}

/** Rust 侧派生好的撤食三态（票 02）。types.ts 的 ColonyAction 尚未收录该字段，
 *  以可选交叉类型收窄——旧载荷/非 follow 块缺字段按 none 兜底。 */
export function retrievalStateOf(
  a: ColonyAction & { retrieval_state?: RetrievalState },
): RetrievalState {
  return a.retrieval_state ?? "none";
}

/**
 * 撤食块展示态（票 02）：三档、无倒计时文字——无待撤置灰不可点（前端 disabled）、
 * 待撤未到期正常可点、逾期红（复用现有超期红样式）。冬眠不静音（发霉不等人）。
 */
export function retrievalTile(
  a: ColonyAction & { retrieval_state?: RetrievalState },
): TileView {
  switch (retrievalStateOf(a)) {
    case "overdue":
      return { tone: "bad", text: "⚠ 该撤食了" };
    case "pending":
      return { tone: "ok", text: "待撤食" };
    default:
      return { tone: "none", text: "无待撤" };
  }
}

/** 喂食 tile 悬停提示：逐食物"距上次"，超期的标出来。 */
export function feedingTooltip(foods: FoodTileInfo[]): string {
  return foods
    .map((f) => {
      const days = f.days_since_last === null ? "尚未记录" : `距上次 ${f.days_since_last} 天`;
      const mark = f.overdue ? " · 超期" : "";
      return `${f.name}：${days}${mark}`;
    })
    .join("\n");
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
