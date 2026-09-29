import { describe, expect, it } from "vitest";
import type { CareActionItem, FoodItem } from "../types";
import {
  buildActionRows,
  buildCategoryIntervalRows,
  buildFoodRows,
  parseInterval,
  toActionInputs,
  toCategoryIntervalInputs,
  toFoodInputs,
  validateActionRows,
  validateCategoryIntervalRows,
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
    category: "seed",
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

  it("follow（跟随喂食，票 01）性质原样进行、原样出 save 入参", () => {
    const rows = buildActionRows([
      actionItem({ id: 5, name: "撤食", kind: "follow", is_preset: true, sort: 2 }),
    ]);
    expect(rows[0]!.kind).toBe("follow");
    expect(toActionInputs(rows)[0]).toMatchObject({ id: 5, name: "撤食", kind: "follow" });
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
  it("buildFoodRows 排序并携带停用/被引用；toFoodInputs 按行下标落 sort、名字 trim（ADR 0008：食物行不再带间隔）", () => {
    const rows = buildFoodRows([
      foodItem({ id: 2, name: "干虾仁", sort: 2 }),
      foodItem({ id: 1, name: "种子", sort: 1, referenced: true }),
      foodItem({ id: 5, name: "面包虫", enabled: false }),
    ]);
    expect(rows.map((r) => r.name)).toEqual(["种子", "干虾仁", "面包虫"]);
    expect(rows[0]!.referenced).toBe(true);
    expect(rows[2]!.enabled).toBe(false);

    rows.push({ id: null, name: " 糖水 ", enabled: true, referenced: false, isPreset: false, perishable: false, retrievalHoursText: "", category: "sugar" });
    expect(toFoodInputs(rows)).toEqual([
      { id: 1, name: "种子", sort: 0, perishable: false, retrieval_hours: null, category: "seed" },
      { id: 2, name: "干虾仁", sort: 1, perishable: false, retrieval_hours: null, category: "seed" },
      { id: 5, name: "面包虫", sort: 2, perishable: false, retrieval_hours: null, category: "seed" },
      { id: null, name: "糖水", sort: 3, perishable: false, retrieval_hours: null, category: "sugar" },
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

  it("buildFoodRows 映射易腐位与撤食间隔（票 01）：缺省字段回退不易腐/空", () => {
    const rows = buildFoodRows([
      foodItem({ id: 1, name: "干虾仁", perishable: true, retrieval_hours: 24 }),
      foodItem({ id: 2, name: "糖水" }),
    ]);
    expect(rows[0]!.perishable).toBe(true);
    expect(rows[0]!.retrievalHoursText).toBe("24");
    expect(rows[1]!.perishable).toBe(false);
    expect(rows[1]!.retrievalHoursText).toBe("");
  });

  it("toFoodInputs 携带易腐入参；关易腐时撤食间隔一律 null（后端归空）", () => {
    const rows = buildFoodRows([
      foodItem({ id: 1, name: "干虾仁", perishable: true, retrieval_hours: 24 }),
      foodItem({ id: 2, name: "糖水" }),
    ]);
    // 开易腐：间隔文本转数字
    expect(toFoodInputs(rows)[0]).toMatchObject({ perishable: true, retrieval_hours: 24 });
    // 关易腐：即使间隔文本有值也归 null（清空语义由行状态与校验共同保证）
    rows[1]!.retrievalHoursText = "999";
    expect(toFoodInputs(rows)[1]).toMatchObject({ perishable: false, retrieval_hours: null });
  });

  it("validateFoodRows 拦空名与重名", () => {
    const rows = buildFoodRows([foodItem({ id: 1, name: "种子" })]);
    expect(validateFoodRows([{ ...rows[0]!, name: "  " }])).toContain("不能为空");
    expect(
      validateFoodRows([
        ...rows,
        { id: null, name: " 种子 ", enabled: true, referenced: false, isPreset: false, perishable: false, retrievalHoursText: "", category: "seed" },
      ]),
    ).toContain("已存在");
    expect(validateFoodRows(rows)).toBe("");
  });

  it("validateCategoryIntervalRows（ADR 0008）：大类周期拦非法值，通过返回空串", () => {
    const rows = buildCategoryIntervalRows([
      { category: "seed", interval_days: 7 },
      { category: "protein", interval_days: 3 },
      { category: "sugar", interval_days: null },
    ]);
    // 载荷乱序也按固定序归位；空串 = 未设
    expect(rows.map((r) => r.category)).toEqual(["seed", "protein", "sugar"]);
    expect(rows.map((r) => r.intervalText)).toEqual(["7", "3", ""]);

    expect(toCategoryIntervalInputs(rows)).toEqual([
      { category: "seed", intervalDays: 7 },
      { category: "protein", intervalDays: 3 },
      { category: "sugar", intervalDays: null },
    ]);

    rows[0]!.intervalText = "0";
    expect(validateCategoryIntervalRows(rows)).toBe(
      "「种子」大类的周期应是不小于 1 的整数天数",
    );
    rows[0]!.intervalText = "abc";
    expect(validateCategoryIntervalRows(rows)).toContain("周期");
    rows[0]!.intervalText = 5; // type=number 输入框给回数字同样放行
    expect(validateCategoryIntervalRows(rows)).toBe("");
    rows[0]!.intervalText = ""; // 空串 = 清空（不按周期提醒），合法
    expect(validateCategoryIntervalRows(rows)).toBe("");
  });

  it("validateFoodRows 易腐撤食间隔矩阵（票 01）：开易腐必填 1–168 整数，关易腐不校验", () => {
    const rows = buildFoodRows([foodItem({ id: 1, name: "面包虫", perishable: true })]);

    // 开易腐 + 空 → 必填提示
    rows[0]!.retrievalHoursText = "";
    expect(validateFoodRows(rows)).toBe("食物「面包虫」开易腐后必须填写撤食间隔（1–168 的整数小时）");

    // 开易腐 + 0 / 负 / 越界 / 非整数 → 区间提示
    for (const bad of ["0", "-3", "169", "abc", "1.5"]) {
      rows[0]!.retrievalHoursText = bad;
      expect(validateFoodRows(rows)).toBe("食物「面包虫」的撤食间隔应是 1–168 的整数小时");
    }

    // 边界内通过：1 / 24 / 168（type=number 给回数字同样放行）
    for (const good of ["1", "24", "168", 24]) {
      rows[0]!.retrievalHoursText = good;
      expect(validateFoodRows(rows)).toBe("");
    }

    // 关易腐：间隔文本无论什么都不拦（保存时归 null）
    rows[0]!.perishable = false;
    rows[0]!.retrievalHoursText = "0";
    expect(validateFoodRows(rows)).toBe("");
  });

  it("食物大类（ADR 0007）：buildFoodRows 携带大类，toFoodInputs 透传、空串转 null", () => {
    const rows = buildFoodRows([
      foodItem({ id: 1, name: "种子", category: "seed" }),
      foodItem({ id: 2, name: "虾干", category: "protein" }),
      foodItem({ id: 3, name: "蜂蜜", category: "sugar" }),
    ]);
    expect(rows.map((r) => r.category)).toEqual(["seed", "protein", "sugar"]);
    expect(toFoodInputs(rows).map((i) => i.category)).toEqual(["seed", "protein", "sugar"]);
    expect(toFoodInputs([{ ...rows[0]!, category: "" }])[0]!.category).toBeNull();
  });

  it("食物大类（ADR 0007）：validateFoodRows 拦缺大类与非法大类", () => {
    const rows = buildFoodRows([foodItem({ id: 1, name: "蟋蟀" })]);
    expect(validateFoodRows([{ ...rows[0]!, category: "" }])).toBe(
      "食物「蟋蟀」必须选择大类（种子/蛋白质/糖水）",
    );
    expect(validateFoodRows([{ ...rows[0]!, category: "fruit" as never }])).toBe(
      "食物「蟋蟀」的大类不合法（应为 种子/蛋白质/糖水）",
    );
    expect(validateFoodRows(rows)).toBe("");
  });
});
