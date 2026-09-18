import { describe, expect, it } from "vitest";
import type { Colony } from "../types";
import { emptyForm, formFromColony, formToInput, validateColonyForm } from "./colonyForm";

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
