/**
 * 与 Rust 端 DTO 对齐的类型（字段 snake_case，serde 默认行为）。
 * 见 src-tauri/src/colony.rs。
 */

export type ColonyStatus = "active" | "hibernating" | "ended";

/** 窝（Rust 已算好饲养天数） */
export interface Colony {
  id: number;
  name: string;
  species: string | null;
  location_id: number | null;
  start_date: string;
  status: ColonyStatus;
  days_raised: number;
}

/** 新建/编辑窝入参 */
export interface ColonyInput {
  name: string;
  species: string | null;
  location_id: number | null;
  start_date: string;
  status: ColonyStatus;
}

/** 地点（含停用的：首页分组仍按它排） */
export interface LocationItem {
  id: number;
  name: string;
  enabled: boolean;
  sort: number;
}

/** 新增(id=null)/修改(id=有值) 地点入参；停用/删除走单独命令 */
export interface LocationInput {
  id: number | null;
  name: string;
  sort: number;
}

export const COLONY_STATUS_LABELS: Record<ColonyStatus, string> = {
  active: "活跃",
  hibernating: "冬眠",
  ended: "已结束",
};
