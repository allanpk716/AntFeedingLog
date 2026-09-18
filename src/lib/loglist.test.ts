import { describe, expect, it } from "vitest";
import {
  LOG_PAGE_SIZE,
  buildLogFilter,
  buildUpdateInput,
  canPickAction,
  canPickFood,
  dictLabel,
  emptyFilterForm,
  filterFormError,
  formatLogTime,
  nextOffset,
  type LogFilterForm,
} from "./loglist";
import type { CareActionItem, FoodItem } from "../types";

function action(overrides: Partial<CareActionItem>): CareActionItem {
  return {
    id: 1,
    name: "喂食",
    icon: null,
    kind: "reminding",
    is_feeding: true,
    suggested_interval_days: 3,
    enabled: true,
    sort: 1,
    referenced: false,
    is_preset: true,
    ...overrides,
  };
}

function food(overrides: Partial<FoodItem>): FoodItem {
  return {
    id: 1,
    name: "种子",
    enabled: true,
    sort: 1,
    suggested_interval_days: null,
    referenced: false,
    is_preset: true,
    ...overrides,
  };
}

describe("筛选拼装（验收 1：组合筛选 + 备注搜索的 IPC 入参）", () => {
  it("空表单 → 全 null 筛选（后端即全量），页大小取默认", () => {
    expect(buildLogFilter(emptyFilterForm())).toEqual({
      colony_id: null,
      action_id: null,
      start: null,
      end: null,
      note_keyword: null,
      limit: LOG_PAGE_SIZE,
      offset: 0,
    });
  });

  it("各维度组合进一个入参；关键词 trim、空串收敛 null", () => {
    const form: LogFilterForm = {
      colonyId: 2,
      actionId: 3,
      start: "2026-09-01",
      end: "2026-09-18",
      keyword: "  面包虫  ",
    };
    expect(buildLogFilter(form, 50)).toEqual({
      colony_id: 2,
      action_id: 3,
      start: "2026-09-01",
      end: "2026-09-18",
      note_keyword: "面包虫",
      limit: LOG_PAGE_SIZE,
      offset: 50,
    });
    expect(
      buildLogFilter({ ...form, keyword: "   " }).note_keyword,
      "纯空白关键词等同未填",
    ).toBeNull();
  });

  it("开始晚于结束 → 前端先拦，不发起查询", () => {
    expect(filterFormError({ ...emptyFilterForm(), start: "2026-09-10", end: "2026-09-01" })).toContain(
      "倒置",
    );
    expect(filterFormError({ ...emptyFilterForm(), start: "2026-09-01", end: "2026-09-01" })).toBe("");
    expect(filterFormError({ ...emptyFilterForm(), start: "2026-09-01", end: "" })).toBe("");
    expect(filterFormError(emptyFilterForm())).toBe("");
  });
});

describe("编辑表单的停用项可选规则（规则 10：原引用可保留，新挂停用拒绝）", () => {
  it("操作下拉：启用可选；停用但恰好是本条记录原操作 → 可保留；其余停用禁选", () => {
    const feed = action({});
    const deadAction = action({ id: 9, name: "降温", enabled: false });
    expect(canPickAction(feed, 9)).toBe(true);
    expect(canPickAction(deadAction, 9), "原操作虽停用可保留").toBe(true);
    expect(canPickAction(deadAction, 2), "新挂停用操作拒绝").toBe(false);
  });

  it("食物 chips：启用可选；停用但被本条记录引用 → 可保留；其余停用禁选", () => {
    const enabled = food({});
    const referencedDead = food({ id: 4, name: "蚕蛹", enabled: false });
    const otherDead = food({ id: 5, name: "糖水", enabled: false });
    const rowFoodIds = [4];
    expect(canPickFood(enabled, rowFoodIds)).toBe(true);
    expect(canPickFood(referencedDead, rowFoodIds), "原引用停用食物可保留").toBe(true);
    expect(canPickFood(otherDead, rowFoodIds), "新挂停用食物拒绝").toBe(false);
    expect(canPickFood(referencedDead, []), "没引用到就按新挂处理").toBe(false);
  });

  it("停用项带「（已停用）」标记", () => {
    expect(dictLabel("蚕蛹", false)).toBe("蚕蛹（已停用）");
    expect(dictLabel("种子", true)).toBe("种子");
  });
});

describe("编辑载荷与展示", () => {
  it("buildUpdateInput：全量字段、备注 trim、时间原样交后端规整", () => {
    expect(
      buildUpdateInput({
        happenedAt: "2026-09-10T08:30",
        actionId: 1,
        foodIds: [2, 4],
        note: "  加餐  ",
      }),
    ).toEqual({
      occurred_at: "2026-09-10T08:30",
      action_id: 1,
      food_ids: [2, 4],
      note: "加餐",
    });
    // 非喂食操作：食物清空数组
    expect(
      buildUpdateInput({ happenedAt: "", actionId: 2, foodIds: [], note: "" }).food_ids,
    ).toEqual([]);
  });

  it("formatLogTime 压成 YYYY-MM-DD HH:MM；脏值原样返回", () => {
    expect(formatLogTime("2026-09-17 21:00:00")).toBe("2026-09-17 21:00");
    expect(formatLogTime("2026-09-17T21:00")).toBe("2026-09-17 21:00");
    expect(formatLogTime("garbage")).toBe("garbage");
  });

  it("nextOffset：还有余量给下一页 offset，取完为 null（按钮隐藏）", () => {
    expect(nextOffset(50, 120)).toBe(50);
    expect(nextOffset(120, 120)).toBeNull();
    expect(nextOffset(0, 0)).toBeNull();
  });
});
