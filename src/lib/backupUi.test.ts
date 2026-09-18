import { describe, expect, it } from "vitest";
import {
  KEEP_COUNT_ERROR,
  autoBackupInactive,
  formatLastBackup,
  validateKeepCount,
} from "./backupUi";

describe("自动备份区：保留份数校验（票 02 验收 4）", () => {
  it("1–365 合法，返回数值", () => {
    expect(validateKeepCount("1")).toBe(1);
    expect(validateKeepCount("30")).toBe(30);
    expect(validateKeepCount("365")).toBe(365);
  });

  it("越界/非正整数/空串拒绝，返回 null", () => {
    expect(validateKeepCount("0")).toBeNull();
    expect(validateKeepCount("366")).toBeNull();
    expect(validateKeepCount("-3")).toBeNull();
    expect(validateKeepCount("3.5")).toBeNull();
    expect(validateKeepCount("abc")).toBeNull();
    expect(validateKeepCount("")).toBeNull();
  });

  it("两端空白容忍", () => {
    expect(validateKeepCount(" 42 ")).toBe(42);
  });

  it("number 输入框给数值（Vue v-model 转型）也接受", () => {
    expect(validateKeepCount(90)).toBe(90);
    expect(validateKeepCount(0)).toBeNull();
    expect(validateKeepCount(366)).toBeNull();
  });

  it("错误提示语含合法范围", () => {
    expect(KEEP_COUNT_ERROR).toContain("1");
    expect(KEEP_COUNT_ERROR).toContain("365");
  });
});

describe("自动备份区：未生效提示判定（票 02 验收 3）", () => {
  it("开关开 + 目录未设 → 未生效", () => {
    expect(autoBackupInactive({ enabled: true, backup_dir: null })).toBe(true);
  });

  it("目录已设 → 不提示", () => {
    expect(autoBackupInactive({ enabled: true, backup_dir: "D:/bk" })).toBe(false);
  });

  it("开关关 → 不提示（关着谈不上未生效）", () => {
    expect(autoBackupInactive({ enabled: false, backup_dir: null })).toBe(false);
  });
});

describe("自动备份区：上次备份状态文案（票 02 验收 5）", () => {
  it("尚未备份", () => {
    expect(formatLastBackup(null)).toBe("尚未备份");
  });

  it("成功 + 时间", () => {
    expect(formatLastBackup({ ok: true, at: "2026-09-18 08:00:00", reason: null })).toBe(
      "成功 · 2026-09-18 08:00:00",
    );
  });

  it("失败 + 时间 + 原因", () => {
    expect(formatLastBackup({ ok: false, at: "2026-09-18 09:00:00", reason: "网盘掉线" })).toBe(
      "失败 · 2026-09-18 09:00:00 · 网盘掉线",
    );
  });

  it("失败但原因缺失 → 按未知原因展示", () => {
    expect(formatLastBackup({ ok: false, at: "2026-09-18 09:00:00", reason: null })).toBe(
      "失败 · 2026-09-18 09:00:00 · 未知原因",
    );
  });
});
