/**
 * 自动备份区展示逻辑（数据安全二期票 02）：保留份数校验、未生效判定、
 * 上次备份状态文案。纯函数，vitest 直测；Rust 侧对应 backup_config.rs（D1/D10/D11）。
 */

/** 保留份数合法范围（票面验收 4：1–365） */
export const KEEP_COUNT_MIN = 1;
export const KEEP_COUNT_MAX = 365;

/** 保留份数非法提示（与 Rust 侧 validate_keep_count 同一口径） */
export const KEEP_COUNT_ERROR = `保留份数须在 ${KEEP_COUNT_MIN}–${KEEP_COUNT_MAX} 之间`;

/**
 * 保留份数输入校验：正整数且在 1–365 内返回数值，否则返回 null（拒绝并提示）。
 * number 输入框的 v-model 会给数值（Vue 自动转型），字符串/数值都收；两端空白容忍。
 */
export function validateKeepCount(value: string | number): number | null {
  const text = String(value).trim();
  if (!/^\d+$/.test(text)) return null;
  const n = Number(text);
  if (n < KEEP_COUNT_MIN || n > KEEP_COUNT_MAX) return null;
  return n;
}

/** 未生效判定（验收 3）：开关开着 ∧ 目录未设。开关关着不提示（关着谈不上未生效）。 */
export function autoBackupInactive(config: { enabled: boolean; backup_dir: string | null }): boolean {
  return config.enabled && !config.backup_dir;
}

/**
 * 上次备份状态文案（验收 5，模板外再包「上次备份：」前缀）：
 * null = 尚未备份；成功+时间；失败+时间+原因（原因缺失按未知原因展示）。
 */
export function formatLastBackup(outcome: { ok: boolean; at: string; reason: string | null } | null): string {
  if (!outcome) return "尚未备份";
  if (outcome.ok) return `成功 · ${outcome.at}`;
  return `失败 · ${outcome.at} · ${outcome.reason ?? "未知原因"}`;
}
