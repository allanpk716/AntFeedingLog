/**
 * 通知设置（票 06）：设置弹窗「通知」tab 的纯逻辑。
 * get_settings / set_settings 走 Rust settings::AppSettings，这里只做
 * 表单态 ↔ 设置态转换与提前天数输入校验（type=number 经 v-model 可能给回
 * 数字或半输入状态文本，统一按 string | number 处理）。
 */

import type { AppSettings } from "../types";

/** 通知 tab 的表单态：提前天数保持文本，便于清空重输 */
export interface NotifySettingsForm {
  master: boolean;
  overdue: boolean;
  hibernation: boolean;
  daysAheadText: string | number;
}

/** 提前天数输入非法时的报错文案 */
export const DAYS_AHEAD_ERROR = "临近出眠提前天数应为 0–365 的整数";

/** 设置态 → 表单态（弹窗打开回显） */
export function toForm(s: AppSettings): NotifySettingsForm {
  return {
    master: s.notify_master_enabled,
    overdue: s.notify_overdue_enabled,
    hibernation: s.notify_hibernation_enabled,
    daysAheadText: String(s.wake_remind_days_ahead),
  };
}

/** 提前天数：非负整数 0–365；非法返回 null */
export function parseDaysAhead(text: string | number): number | null {
  const t = String(text).trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  if (n > 365) return null;
  return n;
}

/**
 * 表单态 → 设置态（保存入参）。提前天数非法返回 null（调用方报错不落库）；
 * autostart 由通知 tab 的「开机自启」开关提供（票 09），随保存一起落库。
 */
export function toSettings(f: NotifySettingsForm, autostart: boolean): AppSettings | null {
  const days = parseDaysAhead(f.daysAheadText);
  if (days === null) return null;
  return {
    notify_master_enabled: f.master,
    notify_overdue_enabled: f.overdue,
    notify_hibernation_enabled: f.hibernation,
    wake_remind_days_ahead: days,
    autostart_enabled: autostart,
  };
}
