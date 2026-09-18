/**
 * 窝表单（新建/编辑共用）的校验与 IPC 入参映射。
 * 规则与 Rust 层一致：名字 trim 后必填且全局唯一（首尾空格差异算重名），
 * 开始日期必须 YYYY-MM-DD。前端先拦一道给出即时反馈，Rust 仍是权威校验。
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
