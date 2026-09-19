/**
 * 窝表单（新建/编辑共用）的校验与 IPC 入参映射。
 * 规则与 Rust 层一致：名字 trim 后必填且全局唯一（首尾空格差异算重名），
 * 开始日期必须 YYYY-MM-DD。前端先拦一道给出即时反馈，Rust 仍是权威校验。
 * 「周期提醒」小节（每窝周期票 04）的行构造与整组校验也在此：周期 1..365
 * 整数天，留空 = 未设；前端先拦一道，Rust set_colony_action_interval 仍权威。
 */

import type { Colony, ColonyInput, ColonyStatus } from "../types";
import { isValidIsoDate } from "./dates";

export interface ColonyForm {
  name: string;
  species: string;
  locationId: number | null;
  startDate: string;
  status: ColonyStatus;
}

export function emptyForm(today: string, defaultLocationId: number | null): ColonyForm {
  return {
    name: "",
    species: "",
    locationId: defaultLocationId,
    startDate: today,
    status: "active",
  };
}

export function formFromColony(colony: Colony): ColonyForm {
  return {
    name: colony.name,
    species: colony.species ?? "",
    locationId: colony.location_id,
    startDate: colony.start_date,
    status: colony.status,
  };
}

/** 返回第一个错误的文案；合法返回 null。editingId 用于编辑时排除自己。 */
export function validateColonyForm(
  form: ColonyForm,
  existing: Colony[],
  editingId: number | null,
): string | null {
  const name = form.name.trim();
  if (!name) {
    return "窝名字不能为空";
  }
  const duplicated = existing.some((c) => c.id !== editingId && c.name.trim() === name);
  if (duplicated) {
    return `窝名字「${name}」已存在，换个名字吧`;
  }
  if (!isValidIsoDate(form.startDate)) {
    return "开始饲养日期格式应为 YYYY-MM-DD";
  }
  return null;
}

/** trim 名字与物种、空物种转 null，键名转 Rust 入参的 snake_case。 */
export function formToInput(form: ColonyForm): ColonyInput {
  const species = form.species.trim();
  return {
    name: form.name.trim(),
    species: species === "" ? null : species,
    location_id: form.locationId,
    start_date: form.startDate.trim(),
    status: form.status,
  };
}

// ── 「周期提醒」小节（每窝周期票 04）─────────────────────────────────────

/** 小节一行：启用的操作 + 周期输入框文本（"" = 未设）。 */
export interface IntervalRowInput {
  actionId: number;
  actionName: string;
  raw: string;
}

/** 保存时的周期入参：intervalDays 数字 = 设/改，null = 清除删行。 */
export interface IntervalSave {
  actionId: number;
  intervalDays: number | null;
}

/**
 * 该窝每个启用操作一行，撤食（follow）不出现。tiles 本就只含启用操作
 * （Rust colony.rs「每个启用中操作一块」），停用操作到不了这里。
 * 已设行回显原始每窝周期值——tiles 契约（types.ts）：interval_from_colony=true
 * 时 effective_interval_days 即原始值；只认旗标，不拿有效周期残留充数。
 */
export function intervalRowsFromColony(colony: Colony): IntervalRowInput[] {
  return colony.actions
    .filter((a) => a.kind !== "follow")
    .map((a) => ({
      actionId: a.action_id,
      actionName: a.name,
      raw:
        a.interval_from_colony === true && typeof a.effective_interval_days === "number"
          ? String(a.effective_interval_days)
          : "",
    }));
}

/** 解析一行周期输入："" = 未设（null）；1..365 整数合法；其余人话报错（口径同 Rust）。 */
export function parseIntervalDays(raw: string): number | null | string {
  const text = raw.trim();
  if (text === "") return null;
  const days = /^\d+$/.test(text) ? Number(text) : Number.NaN;
  if (!Number.isInteger(days) || days < 1 || days > 365) {
    return `每窝周期应是 1–365 的整数天（收到 ${text}）`;
  }
  return days;
}

/**
 * 整组解析并挑出相对回显基线有变化的行（没动的行不打扰后端）；任一行非法
 * 返回人话报错且 saves 为空——调用方见 error 非 null 就整组不发（不落库）。
 */
export function planIntervalSaves(
  rows: IntervalRowInput[],
  original: readonly IntervalRowInput[],
): { saves: IntervalSave[]; error: string | null } {
  const saves: IntervalSave[] = [];
  for (const row of rows) {
    const parsed = parseIntervalDays(row.raw);
    if (typeof parsed === "string") {
      return { saves: [], error: `「${row.actionName}」${parsed}` };
    }
    const before = original.find((o) => o.actionId === row.actionId);
    if (parsed !== parseIntervalDays(before?.raw ?? "")) {
      saves.push({ actionId: row.actionId, intervalDays: parsed });
    }
  }
  return { saves, error: null };
}
