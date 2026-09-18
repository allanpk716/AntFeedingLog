import { describe, expect, it } from "vitest";
import { daysRaised, isValidIsoDate, todayIso } from "./dates";

describe("饲养天数（前端同口径：今天 − 开始饲养日期的自然日数）", () => {
  it("同一天为 0", () => {
    expect(daysRaised("2026-09-18", "2026-09-18")).toBe(0);
  });

  it("跨月按自然日数（与 spec 示例一致：2026-01-20 → 2026-09-18 = 241）", () => {
    expect(daysRaised("2026-01-20", "2026-09-18")).toBe(241);
  });

  it("跨闰日正确", () => {
    expect(daysRaised("2028-02-28", "2028-03-01")).toBe(2);
    expect(daysRaised("2027-02-28", "2027-03-01")).toBe(1);
  });

  it("开始日期在未来为负数（原样返回，展示层自行处理）", () => {
    expect(daysRaised("2026-09-19", "2026-09-18")).toBe(-1);
  });

  it("非法格式抛错", () => {
    expect(() => daysRaised("2026/01/20", "2026-09-18")).toThrow();
    expect(() => daysRaised("2026-01-20", "not-a-date")).toThrow();
    expect(() => daysRaised("2026-02-30", "2026-09-18")).toThrow();
  });
});

describe("ISO 日期校验", () => {
  it("合法与非法样例", () => {
    expect(isValidIsoDate("2026-09-18")).toBe(true);
    expect(isValidIsoDate(" 2026-09-18 ")).toBe(true);
    expect(isValidIsoDate("2026-2-8")).toBe(false);
    expect(isValidIsoDate("2026/09/18")).toBe(false);
    expect(isValidIsoDate("2026-02-30")).toBe(false);
    expect(isValidIsoDate("")).toBe(false);
  });

  it("todayIso 返回本机今天的 YYYY-MM-DD", () => {
    expect(isValidIsoDate(todayIso())).toBe(true);
  });
});
