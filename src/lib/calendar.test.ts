import { describe, expect, it } from "vitest";
import { monthGrid, shiftMinutes, stepTimeField, weekdayShort } from "./calendar";

describe("monthGrid（周一起排）", () => {
  it("2026-09：1 号是周二 → 首格 null，共 1 前置 + 30 天", () => {
    const cells = monthGrid(2026, 9);
    expect(cells).toHaveLength(31);
    expect(cells[0].iso).toBeNull();
    expect(cells[1].iso).toBe("2026-09-01");
    expect(cells[30].iso).toBe("2026-09-30");
  });

  it("2026-08：8-1 是周六 → 5 前置；2024-02（闰）29 天；2023-02 28 天", () => {
    expect(monthGrid(2026, 8).filter((c) => c.iso === null)).toHaveLength(5);
    // 计划原稿用 .at(-1)，但项目 lib 是 ES2020（Array.prototype.at 是 ES2022），vue-tsc 报 TS2550——改尾下标
    const leap = monthGrid(2024, 2);
    const flat = monthGrid(2023, 2);
    expect(leap[leap.length - 1].iso).toBe("2024-02-29");
    expect(flat[flat.length - 1].iso).toBe("2023-02-28");
  });

  it("月份越界抛错（组件层负责钳制）", () => {
    expect(() => monthGrid(2026, 0)).toThrow();
    expect(() => monthGrid(2026, 13)).toThrow();
  });
});

describe("weekdayShort", () => {
  it("2026-09-19 是周六", () => {
    expect(weekdayShort("2026-09-19")).toBe("六");
  });
});

describe("时间步进（value = YYYY-MM-DDTHH:mm）", () => {
  it("shiftMinutes：普通加减、借位跨日回绕", () => {
    expect(shiftMinutes("2026-09-19T20:05", -10)).toBe("2026-09-19T19:55");
    expect(shiftMinutes("2026-09-19T00:05", -10)).toBe("2026-09-18T23:55");
    expect(shiftMinutes("2026-09-19T23:55", 10)).toBe("2026-09-20T00:05");
  });

  it("stepTimeField：分钟算术进位（59+1→下一小时 00），小时 23↔0 回绕，日期不动", () => {
    expect(stepTimeField("2026-09-19T20:05", "h", 1)).toBe("2026-09-19T21:05");
    expect(stepTimeField("2026-09-19T23:05", "h", 1)).toBe("2026-09-19T00:05");
    expect(stepTimeField("2026-09-19T20:59", "m", 1)).toBe("2026-09-19T21:00");
    expect(stepTimeField("2026-09-19T20:00", "m", -1)).toBe("2026-09-19T19:59");
    // 复审定稿语义：步进只调时刻、不跨日（跨日走 ±10分/现在）；00:00 −1分 回到当天 23:59
    expect(stepTimeField("2026-09-19T00:00", "m", -1)).toBe("2026-09-19T23:59");
  });

  it("非法输入原样返回（脏值兜底，不抛错）", () => {
    expect(shiftMinutes("garbage", -10)).toBe("garbage");
    expect(stepTimeField("", "h", 1)).toBe("");
  });
});
