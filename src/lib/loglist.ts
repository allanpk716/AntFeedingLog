/**
 * 记录列表页纯逻辑（票 08）：筛选拼装（空值收敛 null、关键词 trim）、组合校验、
 * 行内编辑表单的停用项可选规则、编辑载荷与展示文案。
 * 数据与校验权威在 Rust（src-tauri/src/care.rs 的 list_logs / update_log），
 * 这里只做入参拼装与展示判定；规则 10 裁定——编辑时原引用的停用操作/食物可保留，
 * 新挂停用项拒绝（后端同口径兜底）。
 */

import type { CareActionItem, Colony, FoodItem, LogFilter, LogUpdateInput } from "../types";

/** 页大小：与 Rust 端 list_logs 缺省一致 */
export const LOG_PAGE_SIZE = 50;

/** 筛选表单原始态（空串 = 未填，null = 未选） */
export interface LogFilterForm {
  locationId: number | null;
  colonyId: number | null;
  actionId: number | null;
  start: string;
  end: string;
  keyword: string;
}

export function emptyFilterForm(): LogFilterForm {
  return { locationId: null, colonyId: null, actionId: null, start: "", end: "", keyword: "" };
}

/** 表单 → IPC 入参：空串/纯空白收敛 null，关键词 trim；offset 供「加载更多」 */
export function buildLogFilter(form: LogFilterForm, offset = 0): LogFilter {
  const kw = form.keyword.trim();
  return {
    location_id: form.locationId,
    colony_id: form.colonyId,
    action_id: form.actionId,
    start: form.start === "" ? null : form.start,
    end: form.end === "" ? null : form.end,
    note_keyword: kw === "" ? null : kw,
    limit: LOG_PAGE_SIZE,
    offset,
  };
}

/** 组合校验：开始晚于结束 → 拦在前端（ISO 日期字典序 = 时间序） */
export function filterFormError(form: LogFilterForm): string {
  if (form.start !== "" && form.end !== "" && form.start > form.end) {
    return "时间范围倒置：开始日期不能晚于结束日期";
  }
  return "";
}

/** 编辑表单操作下拉：启用可选；停用但恰好是本条记录原操作 → 可保留 */
export function canPickAction(a: CareActionItem, currentActionId: number): boolean {
  return a.enabled || a.id === currentActionId;
}

/** 编辑表单食物 chip：启用可选；停用但被本条记录引用 → 可保留（新挂停用拒绝） */
export function canPickFood(f: FoodItem, rowFoodIds: number[]): boolean {
  return f.enabled || rowFoodIds.includes(f.id);
}

/** 下拉/标签文案：停用项带「（已停用）」标记 */
export function dictLabel(name: string, enabled: boolean): string {
  return enabled ? name : `${name}（已停用）`;
}

/** 编辑载荷：全量字段提交（occurred_at 交后端规整 + 拒未来）；备注 trim */
export function buildUpdateInput(args: {
  happenedAt: string;
  actionId: number;
  foodIds: number[];
  note: string;
}): LogUpdateInput {
  return {
    occurred_at: args.happenedAt === "" ? null : args.happenedAt,
    action_id: args.actionId,
    food_ids: args.foodIds,
    note: args.note.trim(),
  };
}

/** 行时间展示："2026-09-17 21:00:00" → "2026-09-17 21:00"；脏值原样返回 */
export function formatLogTime(occurredAt: string): string {
  const s = occurredAt.trim().replace("T", " ");
  return s.length >= 16 ? s.slice(0, 16) : s;
}

/** 「加载更多」的下一页 offset；取完（或空）为 null（按钮隐藏） */
export function nextOffset(loadedCount: number, total: number): number | null {
  return loadedCount < total ? loadedCount : null;
}

/** 地点→窝 级联（交互第三轮 #1）：选了地点，窝下拉只列该地点的窝 */
export function colonyOptionsFor(colonies: Colony[], locationId: number | null): Colony[] {
  if (locationId === null) return colonies;
  return colonies.filter((c) => c.location_id === locationId);
}
