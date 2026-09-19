import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import DatePickerPop from "./DatePickerPop.vue";

const markers = {
  "2026-09-12": { current: true, tip: "清理×1（19:40）· 保湿×1" },
  "2026-09-17": { current: false, tip: "喂食×1（种子）· 保湿×1" },
};

function mountPop(modelValue = "") {
  return mount(DatePickerPop, { props: { modelValue, markers, placeholder: "开始日期" } });
}

function open(w: ReturnType<typeof mountPop>) {
  return w.find(".dp-trigger").trigger("click");
}

describe("DatePickerPop（素版 + 标记层）", () => {
  it("初始关闭；点触发器展开并渲染 2026-09 月格（默认看选中/今天所在月）", async () => {
    const w = mountPop("2026-09-05");
    expect(w.find(".dp-pop").exists()).toBe(false);
    await open(w);
    expect(w.find(".dp-pop").exists()).toBe(true);
    expect(w.find(".dp-title").text()).toBe("2026年9月");
    expect(w.findAll(".dp-day.dim").length).toBe(1); // 2026-09-01 是周二 → 1 个前置格（复审 #5：dim 同带 .dp-day 类）
    expect(w.findAll(".dp-day:not(.dim)").length).toBe(30);
  });

  it("点某天 → 发 update:modelValue 并立即关闭（选中即关）", async () => {
    const w = mountPop("2026-09-05");
    await open(w);
    await w.findAll(".dp-day").find((d) => d.text() === "12")!.trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-12"]);
    expect(w.find(".dp-pop").exists()).toBe(false);
  });

  it("标记：current 日渲染橙点 .dot.cur，其余渲染灰点；悬停 title = tip", async () => {
    const w = mountPop("2026-09-19");
    await open(w);
    const days = w.findAll(".dp-day");
    const d12 = days.find((d) => d.text() === "12")!;
    const d17 = days.find((d) => d.text() === "17")!;
    expect(d12.find(".dot.cur").exists()).toBe(true);
    expect(d12.attributes("title")).toContain("清理×1");
    expect(d17.find(".dot.cur").exists()).toBe(false);
    expect(d17.find(".dot").exists()).toBe(true);
    expect(d17.attributes("title")).toContain("喂食×1");
  });

  it("翻月与「今天」：month 事件随翻页发出；空值挂载从今天所在月开始", async () => {
    const w = mountPop("");
    await open(w);
    // 空值默认今天（测试环境日期不定，只断言能开、有标题）
    expect(w.find(".dp-title").text()).toMatch(/^\d{4}年\d{1,2}月$/);
    await w.find(".dp-prev").trigger("click");
    const evts = w.emitted("month")!;
    // 计划原稿用 .at(-1)，但项目 lib 是 ES2020（Array.prototype.at 是 ES2022），vue-tsc 报 TS2550——改尾下标（同票 02 先例）
    expect(evts[evts.length - 1][0]).toHaveProperty("year");
    await w.find(".dp-today").trigger("click");
    expect(w.find(".dp-title").text()).toMatch(/^\d{4}年\d{1,2}月$/);
  });

  it("无 markers 也能用（素版场景）", async () => {
    const w = mount(DatePickerPop, { props: { modelValue: "2026-09-19" } });
    await open(w);
    expect(w.find(".dp-day .dot").exists()).toBe(false);
  });
});
