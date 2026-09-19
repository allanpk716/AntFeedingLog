/**
 * Pushover 应用内配置（webui-checkin 票 11）纯逻辑：三态来源标签、
 * 明文入库风险提示与占位符文案。输入打码用原生 type=password 切换，
 * 无额外逻辑；三态判定在 Rust pushover::resolve_pushover_credentials。
 */

import type { PushoverSource, PushoverStatus } from "../types";

/** 三态来源标签（设置页「当前生效」标注） */
export const PUSHOVER_SOURCE_LABELS: Record<PushoverSource, string> = {
  app: "应用内配置",
  env: "系统环境变量",
  none: "未配置",
};

/** 状态读取失败（null）时不冒充任何一态 */
export function pushoverSourceLabel(status: PushoverStatus | null): string {
  return status ? PUSHOVER_SOURCE_LABELS[status.source] : "读取失败";
}

/** 明文入库并随备份扩散的风险提示（规格 G：已明示接受，文案固定展示） */
export const PUSHOVER_PLAINTEXT_WARNING =
  "用户键与令牌以明文存入数据库，会随备份文件一起带走，请勿把备份放到不可信的位置。";

/** 占位符：留空回落环境变量（票 11 优先级） */
export const PUSHOVER_PLACEHOLDER = "留空则使用系统环境变量";
