import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import DateTimeField from "./DateTimeField.vue";

function mountField(value = "2026-09-19T20:05") {
  return mount(DateTimeField, { props: { modelValue: value } });
}

describe("DateTimeField（日期+时间组合）", () => {
  it("触发器显示日期+周几；弹层选别的日期 → 发合成后的 datetime（时间部分保留）", async () => {
    const w = mountField("2026-09-19T20:05");
    expect(w.find(".dp-trigger").text()).toContain("2026-09-19");
    expect(w.find(".dtf-wd").text()).toBe("周六"); // 2026-09-19 是周六
    await w.find(".dp-trigger").trigger("click");
    await w.findAll(".dp-day").find((d) => d.text() === "16")!.trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-16T20:05"]);
  });

  it("空值选日期 → 时间兜底 00:00（复审修正）", async () => {
    const w = mountField("");
    await w.find(".dp-trigger").trigger("click");
    await w.findAll(".dp-day").find((d) => d.text() === "16")!.trigger("click");
    // 空值时弹层开在真实今天所在月，期望值不能硬编码月份——只锁日期号 16 与兜底 00:00 语义
    expect(w.emitted("update:modelValue")![0][0] as string).toMatch(/^\d{4}-\d{2}-16T00:00$/);
  });

  // 时间类断言全部「固定基值单步」：mount 无 v-model 回写，props 恒为初值，多步链式断言必假失败
  it("快捷 −10分：基值 20:05 单步 → 19:55", async () => {
    const w = mountField("2026-09-19T20:05");
    await w.find(".dtf-m10").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T19:55"]);
  });

  it("快捷 ＋10分：基值 19:55 单步 → 20:05", async () => {
    const w = mountField("2026-09-19T19:55");
    await w.find(".dtf-p10").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T20:05"]);
  });

  it("快捷 现在：发当前时刻同形态 datetime（YYYY-MM-DDTHH:mm）", async () => {
    const w = mountField("2026-09-19T20:05");
    await w.find(".dtf-now").trigger("click");
    const evts = w.emitted("update:modelValue")!;
    const v = evts[evts.length - 1][0] as string;
    expect(v).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
  });

  it("时分步进显示：num 两段 = 时/分（‹› 按钮字符夹在中间，不能整串断言）", () => {
    const w = mountField("2026-09-19T23:05");
    const nums = w.findAll(".dtf-time .num");
    expect(nums.map((n) => n.text())).toEqual(["23", "05"]);
  });

  it("时步进 h+：23→00 回绕，日期与分钟不动（固定基值单步）", async () => {
    const w = mountField("2026-09-19T23:05");
    await w.find("[data-step='h+']").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T00:05"]);
  });

  it("时步进 h-：00→23 回绕（固定基值单步）", async () => {
    const w = mountField("2026-09-19T00:05");
    await w.find("[data-step='h-']").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T23:05"]);
  });

  it("分步进 m+：59 进位到下一小时 00（固定基值单步）", async () => {
    const w = mountField("2026-09-19T19:59");
    await w.find("[data-step='m+']").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T20:00"]);
  });

  it("分步进 m-：00 借位到 23:59，日期不动（步进不跨日，固定基值单步）", async () => {
    const w = mountField("2026-09-19T00:00");
    await w.find("[data-step='m-']").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-19T23:59"]);
  });

  it("month 事件从 DatePickerPop 转发（挂载初发 + 翻页）", async () => {
    const w = mountField("2026-09-19T20:05");
    await w.find(".dp-trigger").trigger("click");
    await w.find(".dp-prev").trigger("click");
    const evts = w.emitted("month")!;
    expect(evts.length).toBeGreaterThanOrEqual(2);
    // 计划原稿用 .at(-1)，但项目 lib 是 ES2020（Array.prototype.at 是 ES2022），
    // vue-tsc 报 TS2550——改尾下标（同票 02 先例）
    expect(evts[evts.length - 1][0]).toHaveProperty("year");
    expect(evts[evts.length - 1][0]).toHaveProperty("month");
  });
});
