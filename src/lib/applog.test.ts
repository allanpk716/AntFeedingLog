import { describe, expect, it } from "vitest";
import { formatAbnormalExit } from "./applog";

describe("日志区：上次异常退出文案（票 01）", () => {
  it("正常退出（null）不展示", () => {
    expect(formatAbnormalExit(null)).toBeNull();
  });

  it("无崩溃日志 → 标注疑强杀/断电（验收 2 文案）", () => {
    expect(
      formatAbnormalExit({ session_started_at: "2026-09-18 08:00:00", reason: null }),
    ).toBe("上次异常退出：2026-09-18 08:00:00（无崩溃日志，疑强杀/断电）");
  });

  it("有 panic 原因 → 原因行照展示（验收 1）", () => {
    expect(
      formatAbnormalExit({
        session_started_at: "2026-09-18 08:00:00",
        reason: "Rust panic: 库已损坏 @ db.rs:1",
      }),
    ).toBe("上次异常退出：2026-09-18 08:00:00（Rust panic: 库已损坏 @ db.rs:1）");
  });

  it("运行标记损坏 → 时间未知照常展示", () => {
    expect(formatAbnormalExit({ session_started_at: null, reason: null })).toBe(
      "上次异常退出：时间未知（无崩溃日志，疑强杀/断电）",
    );
  });
});
