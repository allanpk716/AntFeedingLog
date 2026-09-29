/**
 * 食物大类（ADR 0007）的展示映射与分组纯函数。
 * 大类固定三种、key 存库（seed/protein/sugar），中文只在展示层；
 * 大类不参与提醒，仅作选择界面的分组容器。IPC 不在此，可被 vitest 直接覆盖。
 */

import type { FoodCategory, FoodItem } from "../types";

/** 分组展示顺序：种子 → 蛋白质 → 糖水（与迁移后的 sort 分组一致） */
export const FOOD_CATEGORY_ORDER: readonly FoodCategory[] = ["seed", "protein", "sugar"];

/** key → 中文标签（后端存英文 key，中文不落库、不受改名影响） */
export const FOOD_CATEGORY_LABELS: Record<FoodCategory, string> = {
  seed: "种子",
  protein: "蛋白质",
  sugar: "糖水",
};

/** key → 中文；未知值原样返回（防御脏数据，正常被后端 CHECK 拦住） */
export function foodCategoryLabel(key: string): string {
  return (FOOD_CATEGORY_LABELS as Record<string, string>)[key] ?? key;
}

export interface FoodCategoryGroup {
  key: string;
  label: string;
  items: FoodItem[];
}

/**
 * 食物列表 → 大类分组（打卡/编辑弹窗一屏直选用）。
 * - 组间按 FOOD_CATEGORY_ORDER 排，组内保持传入顺序（调用方已按 sort、id 排好）；
 * - 空组跳过（整组停用就不渲染组头）；
 * - 未知/缺失大类兜底成尾组（label 用原值，空值显示「未分组」），不丢行。
 */
export function groupFoodsByCategory(foods: readonly FoodItem[]): FoodCategoryGroup[] {
  const buckets = new Map<string, FoodItem[]>();
  for (const f of foods) {
    const key = f.category ?? "";
    const list = buckets.get(key);
    if (list) {
      list.push(f);
    } else {
      buckets.set(key, [f]);
    }
  }
  const groups: FoodCategoryGroup[] = [];
  for (const key of FOOD_CATEGORY_ORDER) {
    const items = buckets.get(key);
    if (items && items.length > 0) {
      groups.push({ key, label: FOOD_CATEGORY_LABELS[key], items });
      buckets.delete(key);
    }
  }
  for (const [key, items] of buckets) {
    groups.push({ key, label: key === "" ? "未分组" : key, items });
  }
  return groups;
}
