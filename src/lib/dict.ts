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

/** 食物行本地编辑态 */
export interface FoodRow {
  id: number | null;
  name: string;
  enabled: boolean;
  /** 预置项禁删可停用（反馈第二轮 F2） */
  isPreset: boolean;
  referenced: boolean;
}

/** 字典（含停用）→ 行列表，按 sort、id 排。 */
export function buildFoodRows(foods: FoodItem[]): FoodRow[] {
  return [...foods]
    .sort((a, b) => a.sort - b.sort || a.id - b.id)
    .map((f) => ({
      id: f.id,
      name: f.name,
      enabled: f.enabled,
      isPreset: f.is_preset,
      referenced: f.referenced,
    }));
}

/** 行列表 → save_food 入参：sort = 行下标，名字 trim。 */
export function toFoodInputs(rows: FoodRow[]): FoodInput[] {
  return rows.map((r, index) => ({ id: r.id, name: r.name.trim(), sort: index }));
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
  return "";
}
