import { describe, expect, it } from "vitest";
import type { Colony, ColonyAction } from "../types";
import {
  emptyForm,
  formFromColony,
  formToInput,
  intervalRowsFromColony,
  parseIntervalDays,
  planIntervalSaves,
  validateColonyForm,
} from "./colonyForm";

const existing: Colony[] = [
  {
    id: 1,
    name: "大头一号",
    species: "大头收获蚁",
    location_id: 1,
    start_date: "2026-01-20",
    status: "active",
    days_raised: 241,
    actions: [],
    recent: [],
    hibernation: null,
    checkin: { latest: null, baseline_date: null, days_since_last: null },
  },
];

function filledForm(name: string) {
  return { ...emptyForm("2026-09-18", 1), name };
}

describe("窝表单校验（与 Rust 层同规则的前端拦截）", () => {
  it("名字必填（trim 后）", () => {
    expect(validateColonyForm(filledForm("   "), existing, null)).toContain("不能为空");
  });

  it("重名被拒，首尾空格差异也算重名", () => {
    expect(validateColonyForm(filledForm("大头一号"), existing, null)).toContain("已存在");
    expect(validateColonyForm(filledForm("  大头一号\t"), existing, null)).toContain("已存在");
  });

  it("编辑时保留自己的名字不算重名，改成别人的算", () => {
    expect(validateColonyForm(filledForm(" 大头一号 "), existing, 1)).toBeNull();
    expect(validateColonyForm(filledForm("大头一号"), existing, 2)).toContain("已存在");
  });

  it("开始日期缺失/非法被拒", () => {
    const bad = { ...filledForm("新窝"), startDate: "" };
    expect(validateColonyForm(bad, existing, null)).toContain("YYYY-MM-DD");
    const bad2 = { ...filledForm("新窝"), startDate: "2026/09/18" };
    expect(validateColonyForm(bad2, existing, null)).toContain("YYYY-MM-DD");
  });

  it("合法表单通过", () => {
    expect(validateColonyForm(filledForm("新窝二号"), existing, null)).toBeNull();
  });
});

describe("表单 ↔ IPC 入参映射", () => {
  it("emptyForm 默认今天开养、默认地点、活跃", () => {
    const form = emptyForm("2026-09-18", 7);
    expect(form).toEqual({
      name: "",
      species: "",
      locationId: 7,
      startDate: "2026-09-18",
      status: "active",
    });
  });

  it("formFromColony 从窝回填（编辑场景）", () => {
    const form = formFromColony(existing[0]);
    expect(form).toEqual({
      name: "大头一号",
      species: "大头收获蚁",
      locationId: 1,
      startDate: "2026-01-20",
      status: "active",
    });
  });

  it("formToInput trim 名字与物种、空物种转 null、键名转 snake_case", () => {
    const input = formToInput({
      name: "  新窝  ",
      species: "   ",
      locationId: null,
      startDate: " 2026-09-18 ",
      status: "hibernating",
    });
    expect(input).toEqual({
      name: "新窝",
      species: null,
      location_id: null,
      start_date: "2026-09-18",
      status: "hibernating",
    });
  });
});

/** 造一个操作 tile（只给小节关心的字段，其余按真实 IPC 形状补齐）。 */
function tile(overrides: Partial<ColonyAction> & { action_id: number; name: string }): ColonyAction {
  return {
    icon: null,
    kind: "log_only",
    is_feeding: false,
    suggested_interval_days: null,
    days_since_last: null,
    overdue: false,
    foods: [],
    ...overrides,
  };
}

const colonyWithActions: Colony = {
  ...existing[0],
  actions: [
    tile({ action_id: 11, name: "喂食", kind: "reminding", is_feeding: true }),
    tile({
      action_id: 12,
      name: "加水",
      interval_from_colony: true,
      effective_interval_days: 7,
    }),
    tile({ action_id: 13, name: "撤食", kind: "follow" }),
    tile({ action_id: 14, name: "打扫" }),
  ],
};

describe("「周期提醒」小节纯逻辑（每窝周期票 04）", () => {
  it("intervalRowsFromColony：启用操作各一行，撤食(follow)不出现；已设行回显原始每窝值，未设为空串", () => {
    expect(intervalRowsFromColony(colonyWithActions)).toEqual([
      { actionId: 11, actionName: "喂食", raw: "" },
      { actionId: 12, actionName: "加水", raw: "7" },
      { actionId: 14, actionName: "打扫", raw: "" },
    ]);
  });

  it("intervalRowsFromColony：无操作窝得空表；interval_from_colony 缺省（未设）不误回显有效周期", () => {
    expect(intervalRowsFromColony(existing[0])).toEqual([]);
    const residue: Colony = {
      ...existing[0],
      actions: [
        // 登记类切性质不清空间隔值：effective 有残留但 interval_from_colony=false → 未设
        tile({ action_id: 21, name: "称重", effective_interval_days: 3 }),
      ],
    };
    expect(intervalRowsFromColony(residue)).toEqual([
      { actionId: 21, actionName: "称重", raw: "" },
    ]);
  });

  it("parseIntervalDays：空串=未设(null)，1..365 整数合法", () => {
    expect(parseIntervalDays("")).toBeNull();
    expect(parseIntervalDays("   ")).toBeNull();
    expect(parseIntervalDays("7")).toBe(7);
    expect(parseIntervalDays(" 7 ")).toBe(7);
    expect(parseIntervalDays("007")).toBe(7);
    expect(parseIntervalDays("1")).toBe(1);
    expect(parseIntervalDays("365")).toBe(365);
  });

  it("parseIntervalDays：0/负数/366/小数/非数字都被人话报错拦下", () => {
    for (const bad of ["0", "-3", "366", "7.5", "abc", "1e2", "+7", "７"]) {
      const err = parseIntervalDays(bad);
      expect(typeof err, `输入 ${bad} 应被拒`).toBe("string");
      expect(err as string).toContain("1–365");
    }
    expect(parseIntervalDays("366")).toContain("366");
  });

  it("planIntervalSaves：只挑相对回显值有变化的行，null=清除", () => {
    const rows = [
      { actionId: 11, actionName: "喂食", raw: "" },
      { actionId: 12, actionName: "加水", raw: "9" },
      { actionId: 14, actionName: "打扫", raw: "" },
    ];
    const original = [
      { actionId: 11, actionName: "喂食", raw: "" },
      { actionId: 12, actionName: "加水", raw: "7" },
      { actionId: 14, actionName: "打扫", raw: "30" },
    ];
    expect(planIntervalSaves(rows, original)).toEqual({
      saves: [
        { actionId: 12, intervalDays: 9 },
        { actionId: 14, intervalDays: null },
      ],
      error: null,
    });
  });

  it("planIntervalSaves：任一行非法即整体报错不落库，报错带上操作名", () => {
    const rows = [
      { actionId: 11, actionName: "喂食", raw: "5" },
      { actionId: 12, actionName: "加水", raw: "400" },
    ];
    const original = [
      { actionId: 11, actionName: "喂食", raw: "" },
      { actionId: 12, actionName: "加水", raw: "" },
    ];
    const plan = planIntervalSaves(rows, original);
    expect(plan.error).toContain("加水");
    expect(plan.error).toContain("1–365");
    expect(plan.saves).toEqual([]);
  });

  it("planIntervalSaves：全部未动一行都不发", () => {
    const rows = [{ actionId: 12, actionName: "加水", raw: "7" }];
    expect(planIntervalSaves(rows, rows)).toEqual({ saves: [], error: null });
  });
});
