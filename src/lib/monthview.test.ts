import { describe, expect, it } from "vitest";
import { buildMarkers, dupWarningText, duplicateInfo } from "./monthview";
import type { MonthDayRecords } from "../types";

const rows: MonthDayRecords[] = [
  { day: 12, action_id: 4, count: 1, last_time: "2026-09-12 19:40:00" },
  { day: 12, action_id: 3, count: 1, last_time: "2026-09-12 21:00:00" },
  { day: 17, action_id: 1, count: 2, last_time: "2026-09-17 20:00:00" },
  { day: 19, action_id: 4, count: 1, last_time: "2026-09-19 14:32:00" },
];
const names = new Map<number, string>([[1, "喂食"], [3, "巢穴保湿"], [4, "垃圾清理"]]);

describe("buildMarkers", () => {
  it("当前操作日 current=true，tip 汇总当天全部操作", () => {
    const m = buildMarkers(rows, 4, names);
    expect(m["2026-09-12"]).toEqual({ current: true, tip: "垃圾清理×1 · 巢穴保湿×1" });
    expect(m["2026-09-17"]).toEqual({ current: false, tip: "喂食×2" });
    expect(m["2026-09-19"]!.current).toBe(true);
    expect(m["2026-09-01"]).toBeUndefined();
  });
});

describe("duplicateInfo", () => {
  it("选中日 + 同操作 → count/最近时刻（HH:MM）；否则 null", () => {
    expect(duplicateInfo(rows, 2026, 9, "2026-09-19", 4)).toEqual({ count: 1, lastTime: "14:32" });
    expect(duplicateInfo(rows, 2026, 9, "2026-09-17", 4)).toBeNull(); // 有其它操作但无当前操作
    expect(duplicateInfo(rows, 2026, 9, "2026-09-01", 4)).toBeNull(); // 无任何记录
  });
  it("非本月日期直接 null（防串月）", () => {
    expect(duplicateInfo(rows, 2026, 9, "2026-08-19", 4)).toBeNull();
  });
});

describe("dupWarningText", () => {
  it("今天与其它日期的文案形态", () => {
    expect(dupWarningText({ count: 1, lastTime: "14:32" }, "2026-09-19", "2026-09-19", "清理"))
      .toBe("今天已有 1 条清理记录（14:32），请确认不是重复操作。");
    expect(dupWarningText({ count: 2, lastTime: "09:05" }, "2026-09-12", "2026-09-19", "喂食"))
      .toBe("9月12日已有 2 条喂食记录（最近 09:05），请确认不是重复操作。");
    expect(dupWarningText(null, "2026-09-01", "2026-09-19", "清理")).toBe("");
  });
});
