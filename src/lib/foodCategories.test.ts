/**
 * 食物大类（ADR 0007）分组纯函数测试：顺序、映射、空组跳过、未知大类兜底。
 */
import { describe, expect, it } from "vitest";
import { FOOD_CATEGORY_LABELS, FOOD_CATEGORY_ORDER, foodCategoryLabel, groupFoodsByCategory } from "./foodCategories";
import type { FoodItem } from "../types";

function f(id: number, category: FoodItem["category"] | undefined, name = `食物${id}`): FoodItem {
  return {
    id, name, enabled: true, sort: id, suggested_interval_days: null,
    is_preset: false, referenced: false, category,
  } as FoodItem;
}

describe("foodCategories", () => {
  it("顺序与映射：种子→蛋白质→糖水，key→中文", () => {
    expect(FOOD_CATEGORY_ORDER).toEqual(["seed", "protein", "sugar"]);
    expect(FOOD_CATEGORY_LABELS).toEqual({ seed: "种子", protein: "蛋白质", sugar: "糖水" });
    expect(foodCategoryLabel("protein")).toBe("蛋白质");
    // 未知 key 原样返回（防御脏数据）
    expect(foodCategoryLabel("junk")).toBe("junk");
  });

  it("分组按 种子→蛋白质→糖水 排，组内保持传入顺序", () => {
    const groups = groupFoodsByCategory([
      f(2, "protein"), f(1, "seed"), f(4, "sugar"), f(3, "protein"),
    ]);
    expect(groups.map((g) => [g.key, g.items.map((i) => i.id)])).toEqual([
      ["seed", [1]],
      ["protein", [2, 3]],
      ["sugar", [4]],
    ]);
  });

  it("空组跳过：整组没有食物就不渲染组头", () => {
    const groups = groupFoodsByCategory([f(1, "seed"), f(2, "protein")]);
    expect(groups.map((g) => g.key)).toEqual(["seed", "protein"]);
  });

  it("未知/缺失大类兜底成尾组，不丢行", () => {
    const groups = groupFoodsByCategory([
      f(1, "sugar"), f(2, "junk" as never), f(3, undefined as never),
    ]);
    expect(groups.map((g) => [g.key, g.label, g.items.map((i) => i.id)])).toEqual([
      ["sugar", "糖水", [1]],
      ["junk", "junk", [2]],
      ["", "未分组", [3]],
    ]);
  });
});
