import { describe, expect, it } from "vitest";
import type { CareActionItem, FoodItem } from "../types";
import {
  buildActionRows,
  buildFoodRows,
  parseInterval,
  toActionInputs,
  toFoodInputs,
  validateActionRows,
  validateFoodRows,
  moveRow,
} from "./dict";

function actionItem(partial: Partial<CareActionItem> & { id: number }): CareActionItem {
  return {
    name: `操作${partial.id}`,
    icon: null,
    kind: "log_only",
    is_feeding: false,
    suggested_interval_days: null,
    enabled: true,
    sort: partial.id,
    referenced: false,
    is_preset: false,
    ...partial,
  };
}

function foodItem(partial: Partial<FoodItem> & { id: number }): FoodItem {
  return {
    name: `食物${partial.id}`,
    enabled: true,
    sort: partial.id,
    referenced: false,
    is_preset: false,
    ...partial,
  };
}

describe("操作行：字典 → 本地编辑态", () => {
  it("buildActionRows 按 sort、id 排序，间隔转输入框文本，停用/被引用原样携带", () => {
    const rows = buildActionRows([
      actionItem({ id: 2, sort: 1, name: "垃圾清理", suggested_interval_days: 7 }),
      actionItem({ id: 1, sort: 1, name: "喂食", kind: "reminding", is_feeding: true, suggested_interval_days: 3 }),
      actionItem({ id: 3, sort: 2, name: "换水", enabled: false, referenced: true }),
    ]);
    expect(rows.map((r) => r.name)).toEqual(["喂食", "垃圾清理", "换水"]);
    expect(rows[0]).toMatchObject({ kind: "reminding", isFeeding: true, intervalText: "3", enabled: true });
    expect(rows[2]).toMatchObject({ enabled: false, referenced: true, intervalText: "" });
  });

  it("预置位映射到行：isPreset 跟随 is_preset", () => {
    const rows = buildActionRows([
      actionItem({ id: 1, name: "喂食", is_preset: true }),
      actionItem({ id: 2, name: "降温", is_preset: false }),
    ]);
    expect(rows[0]!.isPreset).toBe(true);
    expect(rows[1]!.isPreset).toBe(false);
  });
});

describe("操作行：本地编辑态 → save 入参", () => {
  it("toActionInputs 按行下标落 sort，名字 trim，间隔文本转数字或 null", () => {
    const rows = buildActionRows([
      actionItem({ id: 1, name: "喂食", kind: "reminding", is_feeding: true, suggested_interval_days: 3 }),
    ]);
    rows[0]!.name = "  投喂  ";
    rows[0]!.intervalText = " 5 ";
    const inputs = toActionInputs(rows);
    expect(inputs).toEqual([
      { id: 1, name: "投喂", kind: "reminding", is_feeding: true, suggested_interval_days: 5, sort: 0 },
    ]);
    rows[0]!.intervalText = "";
    expect(toActionInputs(rows)[0]!.suggested_interval_days).toBeNull();
  });

  it("parseInterval：空串=null、整数=数值、非法=NaN（交给校验拦）", () => {
    expect(parseInterval("")).toBeNull();
    expect(parseInterval(" 7 ")).toBe(7);
    expect(parseInterval("abc")).toBeNaN();
  });

  it("parseInterval 接受 type=number 输入框经 v-model 给回的数字（回归：数字不炸）", () => {
    expect(parseInterval(3)).toBe(3);
    expect(parseInterval(0)).toBe(0);
    expect(validateActionRows([{
      id: 1, name: "喂食", kind: "reminding", isFeeding: true,
      intervalText: 3, enabled: true, isPreset: true, referenced: false,
    }])).toBe("");
  });
});

describe("操作行：保存前本地预检", () => {
  it("空名 / 重名 / 非法间隔各报一条友好错误，通过返回空串", () => {
    const rows = buildActionRows([
      actionItem({ id: 1, name: "喂食", suggested_interval_days: 3 }),
      actionItem({ id: 2, name: "垃圾清理" }),
    ]);

    rows[0]!.name = "   ";
    expect(validateActionRows(rows)).toContain("不能为空");

    rows[0]!.name = "垃圾清理";
    expect(validateActionRows(rows)).toContain("已存在");

    rows[0]!.name = "投喂";
    rows[0]!.intervalText = "0";
    expect(validateActionRows(rows)).toContain("建议间隔");
    rows[0]!.intervalText = "abc";
    expect(validateActionRows(rows)).toContain("建议间隔");

    rows[0]!.intervalText = "3";
    expect(validateActionRows(rows)).toBe("");
  });
});

describe("操作行排序", () => {
  it("moveRow 在边界内交换、越界不动", () => {
    const rows = buildActionRows([actionItem({ id: 1 }), actionItem({ id: 2 }), actionItem({ id: 3 })]);
    moveRow(rows, 0, -1);
    expect(rows.map((r) => r.name)).toEqual(["操作1", "操作2", "操作3"]);
    moveRow(rows, 0, 1);
    expect(rows.map((r) => r.name)).toEqual(["操作2", "操作1", "操作3"]);
    moveRow(rows, 2, 1);
    expect(rows.map((r) => r.name)).toEqual(["操作2", "操作1", "操作3"]);
  });
});

describe("食物行", () => {
  it("buildFoodRows 排序并携带停用/被引用；toFoodInputs 按行下标落 sort、名字 trim", () => {
    const rows = buildFoodRows([
      foodItem({ id: 2, name: "干虾仁", sort: 2 }),
      foodItem({ id: 1, name: "种子", sort: 1, referenced: true }),
      foodItem({ id: 5, name: "面包虫", enabled: false }),
    ]);
    expect(rows.map((r) => r.name)).toEqual(["种子", "干虾仁", "面包虫"]);
    expect(rows[0]!.referenced).toBe(true);
    expect(rows[2]!.enabled).toBe(false);

    rows.push({ id: null, name: " 糖水 ", enabled: true, referenced: false, isPreset: false });
    expect(toFoodInputs(rows)).toEqual([
      { id: 1, name: "种子", sort: 0 },
      { id: 2, name: "干虾仁", sort: 1 },
      { id: 5, name: "面包虫", sort: 2 },
      { id: null, name: "糖水", sort: 3 },
    ]);
  });

  it("buildFoodRows 映射 isPreset：预置跟随 is_preset", () => {
    const rows = buildFoodRows([
      foodItem({ id: 1, name: "种子", is_preset: true }),
      foodItem({ id: 2, name: "糖水", is_preset: false }),
    ]);
    expect(rows[0]!.isPreset).toBe(true);
    expect(rows[1]!.isPreset).toBe(false);
  });

  it("validateFoodRows 拦空名与重名", () => {
    const rows = buildFoodRows([foodItem({ id: 1, name: "种子" })]);
    expect(validateFoodRows([{ ...rows[0]!, name: "  " }])).toContain("不能为空");
    expect(
      validateFoodRows([...rows, { id: null, name: " 种子 ", enabled: true, referenced: false, isPreset: false }]),
    ).toContain("已存在");
    expect(validateFoodRows(rows)).toBe("");
  });
});
