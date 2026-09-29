/**
 * 字典管理（票 04）的纯逻辑：字典 → 本地编辑态行、行 → save 入参、保存前预检。
 * IPC 全在组件里，这里不碰 Tauri，可被 vitest 直接覆盖。
 * 后端权威校验在 src-tauri/src/dict.rs，这里的预检只为少跑一趟 IPC。
 */

import type {
  ActionInput,
  ActionKind,
  CareActionItem,
  FoodCategory,
  FoodCategoryInterval,
  FoodInput,
  FoodItem,
} from "../types";
import { FOOD_CATEGORY_LABELS, FOOD_CATEGORY_ORDER } from "./foodCategories";

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

// ── 大类周期（ADR 0008）──────────────────────────────────────────────────

/** 大类周期行本地编辑态：间隔用输入框原文承载（空串=未设＝不按周期提醒），
 *  保存时才解析。三类固定、顺序固定（seed→protein→sugar）。 */
export interface CategoryIntervalRow {
  category: FoodCategory;
  /** 展示标签（种子/蛋白质/糖水；中文不落库） */
  label: string;
  intervalText: string | number;
}

/** 后端大类周期（恒三行）→ 行列表；乱序载荷按 FOOD_CATEGORY_ORDER 归位。 */
export function buildCategoryIntervalRows(
  intervals: readonly FoodCategoryInterval[],
): CategoryIntervalRow[] {
  return FOOD_CATEGORY_ORDER.map((category) => ({
    category,
    label: FOOD_CATEGORY_LABELS[category],
    intervalText: (() => {
      const found = intervals.find((i) => i.category === category);
      return found?.interval_days == null ? "" : String(found.interval_days);
    })(),
  }));
}

/** 行列表 → save_food_category_interval 入参（间隔文本转数字或 null）。 */
export function toCategoryIntervalInputs(
  rows: readonly CategoryIntervalRow[],
): { category: FoodCategory; intervalDays: number | null }[] {
  return rows.map((r) => ({
    category: r.category,
    intervalDays: parseInterval(r.intervalText),
  }));
}

/** 大类周期保存前本地预检，返回首个错误文案；通过返回空串。 */
export function validateCategoryIntervalRows(rows: readonly CategoryIntervalRow[]): string {
  for (const r of rows) {
    const interval = parseInterval(r.intervalText);
    if (interval !== null && (!Number.isInteger(interval) || interval < 1)) {
      return `「${r.label}」大类的周期应是不小于 1 的整数天数`;
    }
  }
  return "";
}

// ── 食物 ─────────────────────────────────────────────────────────────────

/** 食物行本地编辑态。
 * 易腐位与撤食间隔（票 01）：撤食间隔同为输入框原文（小时，1–168），
 * 关易腐时清空并置灰、保存一律落 null。
 * ADR 0008 起食物不再有建议间隔（提醒粒度在大类，见 CategoryIntervalRow）。
 * category（ADR 0007）：行内可改；新增行走设置的新增必选下拉，正常不会为空——
 * 类型上仍容空（防夹具/脏数据），validateFoodRows 运行时守护。 */
export interface FoodRow {
  id: number | null;
  name: string;
  enabled: boolean;
  /** 易腐：开启后必须配 1–168 整数小时的撤食间隔（票 01） */
  perishable: boolean;
  /** 撤食间隔输入框原文（小时；空串=未设）；关易腐时被清空置灰 */
  retrievalHoursText: string | number;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  isPreset: boolean;
  referenced: boolean;
  /** 食物大类 key（ADR 0007）：'seed' | 'protein' | 'sugar' */
  category: FoodCategory | "";
}

/** 字典（含停用）→ 行列表，按 sort、id 排。 */
export function buildFoodRows(foods: FoodItem[]): FoodRow[] {
  return [...foods]
    .sort((a, b) => a.sort - b.sort || a.id - b.id)
    .map((f) => ({
      id: f.id,
      name: f.name,
      enabled: f.enabled,
      perishable: f.perishable ?? false,
      retrievalHoursText: f.retrieval_hours == null ? "" : String(f.retrieval_hours),
      isPreset: f.is_preset,
      referenced: f.referenced,
      category: f.category ?? "",
    }));
}

/** 行列表 → save_food 入参：sort = 行下标，名字 trim。
 * 撤食间隔（票 01）：开易腐按文本解析（空串=null、非法=NaN，交给校验拦），
 * 关易腐一律 null（后端兜底归空）。 */
export function toFoodInputs(rows: FoodRow[]): FoodInput[] {
  return rows.map((r, index) => ({
    id: r.id,
    name: r.name.trim(),
    sort: index,
    perishable: r.perishable,
    retrieval_hours: r.perishable ? parseInterval(r.retrievalHoursText) : null,
    category: r.category === "" ? null : r.category,
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
    // 大类必选守护（ADR 0007，与后端 save_food 同口径）：正常经新增下拉必有值，
    // 这里拦夹具缺口/脏数据绕过
    if (r.category === "") {
      return `食物「${label}」必须选择大类（种子/蛋白质/糖水）`;
    }
    if (!(FOOD_CATEGORY_ORDER as readonly string[]).includes(r.category)) {
      return `食物「${label}」的大类不合法（应为 种子/蛋白质/糖水）`;
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
