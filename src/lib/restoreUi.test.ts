import { describe, expect, it } from "vitest";
import type { RestoreSummary } from "../types";
import {
  RESTORE_CONFIRM_TEXT,
  formatBackupDate,
  formatBackupDirInBackup,
  formatRestoreOutcome,
  summaryLooksSuspicious,
  summaryRows,
} from "./restoreUi";

function summary(overrides: Partial<RestoreSummary> = {}): RestoreSummary {
  return {
    backup_date: "2026-09-10",
    colony_count: 2,
    log_count: 5,
    backup_dir_in_backup: null,
    ...overrides,
  };
}

describe("恢复摘要展示（数据安全二期票 04）", () => {
  it("固定确认文案同时点明替换语义与备份设置不回滚（D5/D1）", () => {
    expect(RESTORE_CONFIRM_TEXT).toContain("整体替换");
    expect(RESTORE_CONFIRM_TEXT).toContain("备份设置保持当前值");
    // 评审 R1：窗口期写入会被恢复覆盖丢弃，确认文案如实点明
    expect(RESTORE_CONFIRM_TEXT).toContain("恢复期间新产生的记录不会保留");
  });

  it("摘要行按固定顺序展示四项（验收 1 的数据面）", () => {
    const rows = summaryRows(summary({ backup_dir_in_backup: "D:/old-bk" }));
    expect(rows.map(([label]) => label)).toEqual([
      "备份日期",
      "窝数",
      "记录数",
      "备份内备份目录设置",
    ]);
    expect(rows[0][1]).toBe("2026-09-10");
    expect(rows[1][1]).toBe("2");
    expect(rows[2][1]).toBe("5");
    expect(rows[3][1]).toContain("D:/old-bk");
  });

  it("备份内无目录设置（D1 后的正常备份）→ 点明存库外不随恢复回滚", () => {
    expect(formatBackupDirInBackup(null)).toContain("备份设置存库外");
    expect(formatBackupDirInBackup(null)).toContain("不随恢复回滚");
  });

  it("备份日期缺失（空备份）→ 如实提示未知而非显示空串", () => {
    expect(formatBackupDate(null)).toContain("未知");
    expect(formatBackupDate("2026-01-02")).toBe("2026-01-02");
  });

  it("0 窝 0 条标记疑点（D6 最后防线：选错文件时摘要能暴露）", () => {
    expect(summaryLooksSuspicious(summary({ colony_count: 0, log_count: 0 }))).toBe(true);
    expect(summaryLooksSuspicious(summary())).toBe(false);
    expect(summaryLooksSuspicious(summary({ colony_count: 0, log_count: 3 }))).toBe(false);
  });

  it("结果文案两态：done 刷新 / needs_restart 提示重启（spec D5 附录 #10）", () => {
    expect(formatRestoreOutcome("done")).toContain("已刷新");
    expect(formatRestoreOutcome("done_needs_restart")).toContain("重启");
  });
});
