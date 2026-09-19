/**
 * 字典管理（票 04）的纯逻辑：字典 → 本地编辑态行、行 → save 入参、保存前预检。
 * IPC 全在组件里，这里不碰 Tauri，可被 vitest 直接覆盖。
 * 后端权威校验在 src-tauri/src/dict.rs，这里的预检只为少跑一趟 IPC。
 */

import type { ActionInput, ActionKind, CareActionItem, FoodInput, FoodItem } from "../types";

// ── 操作 ─────────────────────────────────────────────────────────────────

/** 操作行本地编辑态：间隔用输入框原文承载（空串=未设），保存时才解析。
 * type=number 的输入框经 v-model 可能给回数字，故 intervalText 放宽为 string | number。 */
export interface ActionRow {
  id: number | null;
  name: string;
  kind: ActionKind;
  isFeeding: boolean;
  intervalText: string | number;
  enabled: boolean;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  isPreset: boolean;
  referenced: boolean;
}

/** 字典（含停用）→ 行列表，按 sort、id 排（与后端 ORDER BY 同口径）。 */
export function buildActionRows(actions: CareActionItem[]): ActionRow[] {
  return [...actions]
    .sort((a, b) => a.sort - b.sort || a.id - b.id)
    .map((a) => ({
      id: a.id,
      name: a.name,
      kind: a.kind,
      isFeeding: a.is_feeding,
      intervalText: a.suggested_interval_days === null ? "" : String(a.suggested_interval_days),
      enabled: a.enabled,
      isPreset: a.is_preset,
      referenced: a.referenced,
    }));
}

/** 间隔输入框原文 → 数字；空串=null（无建议间隔）；非数字=NaN（交给校验拦）。
 * type=number 输入框经 v-model 可能给回数字，先统一转字符串再判。 */
export function parseInterval(text: string | number): number | null {
  const trimmed = String(text ?? "").trim();
  if (trimmed === "") {
    return null;
  }
  const n = Number(trimmed);
  return Number.isInteger(n) ? n : NaN;
}

/** 行列表 → save_action 入参：sort = 行下标（保存即按当前行序重排），名字 trim。 */
export function toActionInputs(rows: ActionRow[]): ActionInput[] {
  return rows.map((r, index) => ({
    id: r.id,
    name: r.name.trim(),
    kind: r.kind,
    is_feeding: r.isFeeding,
    suggested_interval_days: parseInterval(r.intervalText),
    sort: index,
  }));
}

/** 保存前本地预检，返回首个错误文案；通过返回空串。 */
export function validateActionRows(rows: ActionRow[]): string {
  const names = rows.map((r) => r.name.trim());
  if (names.some((n) => !n)) {
    return "操作名字不能为空";
  }
  const duplicated = names.find((n, i) => names.indexOf(n) !== i);
  if (duplicated) {
    return `操作名字「${duplicated}」已存在`;
  }
  for (const r of rows) {
    const interval = parseInterval(r.intervalText);
    if (interval !== null && (!Number.isInteger(interval) || interval < 1)) {
      return `操作「${r.name.trim()}」的建议间隔应是不小于 1 的整数天数`;
    }
  }
  return "";
}

/** 行内上移/下移（越界不动）；就地修改传入数组。 */
export function moveRow<T>(rows: T[], index: number, direction: -1 | 1): void {
  const target = index + direction;
  if (target < 0 || target >= rows.length) {
    return;
  }
  const tmp = rows[index]!;
  rows[index] = rows[target]!;
  rows[target] = tmp;
}

// ── 食物 ─────────────────────────────────────────────────────────────────

/** 食物行本地编辑态：间隔用输入框原文承载（空串=未设，F3），保存时才解析。
 * 易腐位与撤食间隔（票 01）：撤食间隔同为输入框原文（小时，1–168），
 * 关易腐时清空并置灰、保存一律落 null。 */
export interface FoodRow {
  id: number | null;
  name: string;
  enabled: boolean;
  /** 建议间隔输入框原文（空串=未设）；type=number 可能给回数字，同 ActionRow 口径 */
  intervalText: string | number;
  /** 易腐：开启后必须配 1–168 整数小时的撤食间隔（票 01） */
  perishable: boolean;
  /** 撤食间隔输入框原文（小时；空串=未设）；关易腐时被清空置灰 */
  retrievalHoursText: string | number;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  isPreset: boolean;
  referenced: boolean;
}

/** 字典（含停用）→ 行列表，按 sort、id 排。
 * perishable / retrieval_hours 为可选读取（Rust Food DTO 读取侧由后续票接通），
 * 缺省回退不易腐 / 空。 */
export function buildFoodRows(foods: FoodItem[]): FoodRow[] {
  return [...foods]
    .sort((a, b) => a.sort - b.sort || a.id - b.id)
    .map((f) => ({
      id: f.id,
      name: f.name,
      enabled: f.enabled,
      intervalText: f.suggested_interval_days === null ? "" : String(f.suggested_interval_days),
      perishable: f.perishable ?? false,
      retrievalHoursText: f.retrieval_hours == null ? "" : String(f.retrieval_hours),
      isPreset: f.is_preset,
      referenced: f.referenced,
    }));
}

/** 行列表 → save_food 入参：sort = 行下标，名字 trim，间隔文本转数字或 null（F3）。
 * 撤食间隔（票 01）：开易腐按文本解析（空串=null、非法=NaN，交给校验拦），
 * 关易腐一律 null（后端兜底归空）。 */
export function toFoodInputs(rows: FoodRow[]): FoodInput[] {
  return rows.map((r, index) => ({
    id: r.id,
    name: r.name.trim(),
    sort: index,
    suggested_interval_days: parseInterval(r.intervalText),
    perishable: r.perishable,
    retrieval_hours: r.perishable ? parseInterval(r.retrievalHoursText) : null,
  }));
}

/** 保存前本地预检，返回首个错误文案；通过返回空串。 */
export function validateFoodRows(rows: FoodRow[]): string {
  const names = rows.map((r) => r.name.trim());
  if (names.some((n) => !n)) {
    return "食物名字不能为空";
  }
  const duplicated = names.find((n, i) => names.indexOf(n) !== i);
  if (duplicated) {
    return `食物名字「${duplicated}」已存在`;
  }
  for (const r of rows) {
    const label = r.name.trim();
    const interval = parseInterval(r.intervalText);
    if (interval !== null && (!Number.isInteger(interval) || interval < 1)) {
      return `食物「${label}」的建议间隔应是不小于 1 的整数天数`;
    }
    // 易腐撤食间隔守护（票 01，与后端 save_food 同款）：开易腐必填 1–168 整数小时
    if (r.perishable) {
      const hours = parseInterval(r.retrievalHoursText);
      if (hours === null) {
        return `食物「${label}」开易腐后必须填写撤食间隔（1–168 的整数小时）`;
      }
      if (Number.isNaN(hours) || hours < 1 || hours > 168) {
        return `食物「${label}」的撤食间隔应是 1–168 的整数小时`;
      }
    }
  }
  return "";
}
