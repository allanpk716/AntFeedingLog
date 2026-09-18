/**
 * 日志区展示逻辑（数据安全二期票 01）：上次异常退出的文案拼装。
 * 纯函数，vitest 直测；Rust 侧对应 applog.rs（D7/D8）。
 */
import type { AbnormalExitInfo } from "../types";

/**
 * 上次异常退出的展示文案；正常退出（null）不展示返回 null。
 * - 无原因（强杀/断电，物理上无崩溃日志）：`上次异常退出：<时间>（无崩溃日志，疑强杀/断电）`
 * - 有原因（上次会话有 panic 记录）：`上次异常退出：<时间>（<原因>）`
 * - 时间未知（运行标记损坏）：`上次异常退出：时间未知（…）`
 */
export function formatAbnormalExit(info: AbnormalExitInfo | null): string | null {
  if (!info) return null;
  const when = info.session_started_at ?? "时间未知";
  const why = info.reason ?? "无崩溃日志，疑强杀/断电";
  return `上次异常退出：${when}（${why}）`;
}
