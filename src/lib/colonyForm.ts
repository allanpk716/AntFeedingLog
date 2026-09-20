/**
 * 窝表单（新建/编辑共用）的校验与 IPC 入参映射。
 * 规则与 Rust 层一致：名字 trim 后必填且全局唯一（首尾空格差异算重名），
 * 开始日期必须 YYYY-MM-DD。前端先拦一道给出即时反馈，Rust 仍是权威校验。
 * 「周期提醒」小节（每窝周期票 04）的行构造与整组校验也在此：周期 1..365
 * 整数天，留空 = 未设；前端先拦一道，Rust set_colony_action_interval 仍权威。
 * 保湿方式（保湿方式票 02）的锚名/默认天数与「选/切方式 → 天数框」会话状态
 * 矩阵纯函数也在此；保湿行的落库走 create/update_colony 整窗新通道（非逐行
 * set_colony_action_interval），见 ColonyFormDialog。
 */

import type { Colony, ColonyInput, ColonyStatus, HydrationMethod } from "../types";
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

// ── 保湿方式（保湿方式票 02，规格状态矩阵四态九规则）─────────────────────

/**
 * 巢穴保湿操作的锚点名（与 Rust colony.rs HYDRATION_ACTION_NAME 同名锚）。
 * 改名/停用后匹配不到为已知边界（同 v4/v6/v8 按名回填先例）：编辑退回纯
 * 数字行、方式原样往返；新建只能设方式标签、不落初始周期。
 */
export const HYDRATION_ACTION_NAME = "巢穴保湿";

/** 方式默认天数（spec D2 钉死：手动加水 7 / 水塔 15，不做全局配置项）。 */
export const HYDRATION_DEFAULT_DAYS: Record<HydrationMethod, number> = {
  manual: 7,
  tower: 15,
};

/**
 * 保湿行天数框的会话态：raw = 输入框文本；prefill 非 null 表示框内是本次会话
 * 预填的建议值且未被手工编辑过——方式切换是否跟随以此判定（规格规则 3）。
 */
export interface HydrationSessionState {
  raw: string;
  prefill: number | null;
}

/**
 * 显式选择/切换到某方式后的天数框状态（规则 2/3 + F5 空框切换）：
 * - 空框（含 D 态空框直接切换）→ 预填新方式默认；
 * - 框内是本次会话预填值（未手工改）→ 跟随换新方式默认；
 * - 库内已有周期或手工改过 → 不动，绝不覆盖（规则 2「永不覆盖」）。
 */
export function hydrationAfterPick(
  state: HydrationSessionState,
  method: HydrationMethod,
): HydrationSessionState {
  if (state.raw.trim() === "" || state.prefill !== null) {
    const days = HYDRATION_DEFAULT_DAYS[method];
    return { raw: String(days), prefill: days };
  }
  return state;
}

/**
 * 显式清回未设后的天数框状态（规则 4 + F4 净零）：
 * - 库内方式原值非未设（C 态）→ 框清空（后端同事务删周期行，置灰由组件按
 *   locked 派生）；会话预填值一并消失；
 * - 库内原值 = 未设（B/A 态会话内「选了又改回」）→ 净效果为零：库内值/手工
 *   值原样保留，预填值随方式选择一起消失、不构成落库值。
 */
export function hydrationAfterClear(
  state: HydrationSessionState,
  storedMethod: HydrationMethod | null,
): HydrationSessionState {
  if (storedMethod !== null || state.prefill !== null) {
    return { raw: "", prefill: null };
  }
  return state;
}

/**
 * 保湿行天数的整组校验（口径同 parseIntervalDays，报错带上锚名操作名）：
 * 合法返回天数或 null（未设），非法返回人话报错——调用方见字符串就整组不发。
 */
export function parseHydrationDays(raw: string): number | null | string {
  const parsed = parseIntervalDays(raw);
  return typeof parsed === "string" ? `「${HYDRATION_ACTION_NAME}」${parsed}` : parsed;
}
