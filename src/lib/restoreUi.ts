/**
 * 恢复流程展示逻辑（数据安全二期票 04）：摘要行拼装、确认固定文案、结果文案。
 * 纯函数，vitest 直测；Rust 侧对应 restore.rs（spec D5/D6/D10/D11）。
 */
import type { RestoreSummary } from "../types";

/**
 * 确认弹窗固定文案（spec D5/D1：替换语义 + 备份设置不回滚，两者都要点明；
 * 评审 R1 补第三点：恢复期间新产生的记录不会保留——如实告知覆盖丢弃语义）。
 */
export const RESTORE_CONFIRM_TEXT =
  "恢复将整体替换当前全部数据；备份设置保持当前值，不随恢复回滚；恢复期间新产生的记录不会保留";

/**
 * 备份内备份目录设置值的展示：有旧值回显（点明恢复后备份将流向旧位置的风险）；
 * 无此设置（D1 后的正常备份）点明备份设置存库外、不随恢复回滚。
 */
export function formatBackupDirInBackup(value: string | null): string {
  if (value) return `${value}（恢复不会采用此旧值）`;
  return "备份内无此设置（备份设置存库外，不随恢复回滚）";
}

/** 备份日期展示：null = 库内无记录可判（空库），如实提示。 */
export function formatBackupDate(date: string | null): string {
  return date ?? "未知（备份内没有记录）";
}

/**
 * 摘要预览的展示行（顺序固定：备份日期 / 窝数 / 记录数 / 备份内目录设置值）。
 * 每行 [标签, 值]；0 窝 0 条如实展示——那是选错文件的最后防线信号（D6）。
 */
export function summaryRows(s: RestoreSummary): Array<[string, string]> {
  return [
    ["备份日期", formatBackupDate(s.backup_date)],
    ["窝数", String(s.colony_count)],
    ["记录数", String(s.log_count)],
    ["备份内备份目录设置", formatBackupDirInBackup(s.backup_dir_in_backup)],
  ];
}

/**
 * 摘要是否暴露选错文件的疑点（D6：0 窝 0 条的空备份值得用户多看一眼）。
 * 仅用于给摘要行补一条提醒文案，不拦截（校验链已挡住损坏/未来版本）。
 */
export function summaryLooksSuspicious(s: RestoreSummary): boolean {
  return s.colony_count === 0 && s.log_count === 0;
}

/**
 * 恢复执行结果文案：done = 当场刷新；done_needs_restart = 新库已就位但重开
 * 连接失败（spec D5 附录 #10 降级路径），提示重启。
 */
export function formatRestoreOutcome(outcome: string): string {
  if (outcome === "done") return "恢复完成，界面已刷新";
  return "恢复已完成，请重启应用后生效";
}
