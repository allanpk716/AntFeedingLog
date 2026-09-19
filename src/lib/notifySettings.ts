/**
 * 通知设置（票 06；票 05 增「撤食提醒」开关，webui-checkin 票 11 增 Pushover
 * 应用内凭据两字段）：设置弹窗「通知」tab 的纯逻辑。get_settings / set_settings
 * 走 Rust settings::AppSettings，这里只做表单态 ↔ 设置态转换与提前天数输入校验
 *（type=number 经 v-model 可能给回数字或半输入状态文本，统一按 string | number 处理）。
 */

import type { AppSettings } from "../types";

/**
 * Rust AppSettings + 撤食开关（票 05）。types.ts 的 AppSettings 尚未含该键，
 * 本地交叉类型补形（Rust 端 settings::AppSettings 已带该字段并随 invoke 序列化）；
 * 键缺省（旧数据/迁移期）按开兜底。
 */
export type NotifySettingsModel = AppSettings & { notify_retrieval_enabled?: boolean };

/** 通知 tab 的表单态：提前天数保持文本，便于清空重输；凭据两值原样回显（打码在 UI 层） */
export interface NotifySettingsForm {
  master: boolean;
  /** 撤食提醒开关（票 05）：只闸撤食这一类，不影响其它提醒 */
  retrieval: boolean;
  daysAheadText: string | number;
  pushoverUser: string;
  pushoverToken: string;
}

/** 提前天数输入非法时的报错文案 */
export const DAYS_AHEAD_ERROR = "临近出眠提前天数应为 0–365 的整数";

/** 设置态 → 表单态（弹窗打开回显） */
export function toForm(s: NotifySettingsModel): NotifySettingsForm {
  return {
    master: s.notify_master_enabled,
    retrieval: s.notify_retrieval_enabled ?? true,
    daysAheadText: String(s.wake_remind_days_ahead),
    pushoverUser: s.pushover_user,
    pushoverToken: s.pushover_token,
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
 * 分类子开关作废（反馈第二轮 Q7/Q9）：超期/冬眠两键保留在库里，行为由总开关
 * 统一，固定回写 true；撤食开关（票 05）是唯一仍生效的分类开关，原样落库。
 * 凭据两值原样透传（票 11；trim 在 Rust 保存侧统一做）。
 */
export function toSettings(f: NotifySettingsForm, autostart: boolean): NotifySettingsModel | null {
  const days = parseDaysAhead(f.daysAheadText);
  if (days === null) return null;
  return {
    notify_master_enabled: f.master,
    notify_overdue_enabled: true,
    notify_hibernation_enabled: true,
    notify_retrieval_enabled: f.retrieval,
    wake_remind_days_ahead: days,
    autostart_enabled: autostart,
    pushover_user: f.pushoverUser,
    pushover_token: f.pushoverToken,
  };
}
