/**
 * 与 Rust 端 DTO 对齐的类型（字段 snake_case，serde 默认行为）。
 * 见 src-tauri/src/colony.rs。
 */

export type ColonyStatus = "active" | "hibernating" | "ended";

/** 维护操作性质：reminding=提醒（超期标红可通知）/ log_only=仅登记（永不催促） */
export type ActionKind = "reminding" | "log_only";

/** 窝卡片上单个操作块（Rust 已算好距上次/超期态） */
export interface ColonyAction {
  action_id: number;
  name: string;
  icon: string | null;
  kind: ActionKind;
  /** 是否喂食类操作（schema 标记位，决定记账时是否带食物多选；与名字无关） */
  is_feeding: boolean;
  suggested_interval_days: number | null;
  /** 今天 − 最近一次发生日期（自然日）；从未记录为 null */
  days_since_last: number | null;
  /** 仅提醒类且 > 建议间隔 */
  overdue: boolean;
}

/** 最近记录摘要的一行（前端拼展示文案） */
export interface RecentLog {
  happened_at: string;
  action_name: string;
  food_names: string[];
}

/** 窝（Rust 已算好饲养天数、操作块、最近摘要） */
export interface Colony {
  id: number;
  name: string;
  species: string | null;
  location_id: number | null;
  start_date: string;
  status: ColonyStatus;
  days_raised: number;
  actions: ColonyAction[];
  recent: RecentLog[];
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

/** 食物（含停用的：新建入口前端过滤 enabled；referenced=被历史引用，只能停用不能删） */
export interface FoodItem {
  id: number;
  name: string;
  enabled: boolean;
  sort: number;
  referenced: boolean;
}

/** 维护操作字典项（含停用的；referenced=被记录/提醒台账引用，只能停用不能删） */
export interface CareActionItem {
  id: number;
  name: string;
  icon: string | null;
  kind: ActionKind;
  /** 是否喂食类操作（可编辑、不强制全局唯一，界面提示自理） */
  is_feeding: boolean;
  suggested_interval_days: number | null;
  enabled: boolean;
  sort: number;
  referenced: boolean;
}

/** 新增(id=null)/修改(id=有值) 操作入参；停用/删除走单独命令 */
export interface ActionInput {
  id: number | null;
  name: string;
  kind: ActionKind;
  is_feeding: boolean;
  suggested_interval_days: number | null;
  sort: number;
}

/** set_action_policy 入参：性质 + 建议间隔（null=保留现值）+ 可选 is_feeding（null=不改） */
export interface ActionPolicyInput {
  kind: ActionKind;
  suggested_interval_days: number | null;
  is_feeding: boolean | null;
}

/** 新增(id=null)/修改(id=有值) 食物入参；停用/删除走单独命令 */
export interface FoodInput {
  id: number | null;
  name: string;
  sort: number;
}

/** 记账入参：喂食才带 food_ids（其余操作传空数组）；happened_at 可补录过去 */
export interface CareLogInput {
  colony_id: number;
  action_id: number;
  happened_at: string;
  note: string | null;
  food_ids: number[];
}

export const COLONY_STATUS_LABELS: Record<ColonyStatus, string> = {
  active: "活跃",
  hibernating: "冬眠",
  ended: "已结束",
};
