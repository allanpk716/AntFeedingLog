/**
 * 网页端升级 UI 纯逻辑（webui-update 票 02，规格 Implementation Decisions D）：
 * 升级横幅 / 确认面板 / 升级状态机的视图映射与文案。Rust 契约与桌面同一套
 * （BadgeDetailState / InstallOutcome / Err 固定串，见 src-tauri/src/updater.rs
 * 与 webui_server.rs 派发臂）；文案口径能复用就 import 桌面 updaterUi.ts 同款
 * （installFailedText / isInstallInProgressError / installInProgressText），不复制不改。
 *
 * 状态机（组件侧 phase）：
 *   idle → banner（有新版待点）→ panel（确认面板）→ busy（确认中·命令在途）
 *   → waiting-restart（install_started，等服务端重启）
 *   → restarting（探活失败，服务端已退出）→（SSE 新 epoch）→ 整页刷新拿新 bundle
 * 四条出口：
 *   出口1 install_failed 在 HTTP 生命周期内返回 → 切回 failed（重试可用；
 *         /api/cmd 超时 300 秒，失败响应能活着回来——前端只按返回处理，不自加超时）
 *   出口2 确认起 10 分钟内没等到服务端重启 → restart-timeout（页面手动刷新，无重试）
 *   出口3 Err「远端已没有比当前更新的版本」→ failed 红条不弹重试，收横幅、重读红点
 *   出口4 刷新落地后 sessionStorage 目标版本 vs bundle 版本（package.json）：
 *         相等弹「已升级到 vX」成功提示；不等（旧版本回来了）不弹；两者都清记录
 *   防重入 Err（isInstallInProgressError 口径）→ 平静「进行中」提示，不出重试
 */
import {
  installFailedText,
  installInProgressText,
  isInstallInProgressError,
} from "./updaterUi";
import type { InstallOutcome } from "./updaterUi";

/** 红点详情（get_update_badge_detail 返回；Rust updater::BadgeDetailState，
 *  serde null 不省略）。与 ipc.ts 的导出同形——此处为纯逻辑层自持的形状
 *  （组件把响应直接喂给 bannerForDetail，不经第二道类型转换）。 */
export interface UpdateBadgeDetail {
  available: boolean;
  version: string | null;
  notes: string | null;
}

/** 确认结果的展示视图：Ok 两态 + Err 三分类（防重入 / 复查无新版 / 其余失败） */
export type ConfirmView =
  | { kind: "waiting-restart"; version: string }
  | { kind: "failed"; text: string; retryable: boolean }
  | { kind: "install-in-progress"; text: string }
  | { kind: "no-newer"; text: string };

/** 复查无新版的固定串（updater.rs fresh_update 的 ok_or_else 文案，钉死契约） */
const NO_NEWER_VERSION_MARKER = "远端已没有比当前更新的版本";

/** 识别复查无新版（横幅过期：别处已升级 / 服务端清单已撤） */
export function isNoNewerVersionError(message: string): boolean {
  return message.includes(NO_NEWER_VERSION_MARKER);
}

/**
 * 确认结果 → 展示视图。outcome 为 string = invoke 的 Err（httpInvoke 以响应体
 * {error} 字符串 reject，与桌面同形）；fallbackVersion = 横幅上的目标版本，
 * 给 Err 文案兜底带上版本。防重入拒绝 = 安装健康进行中，映射平静态不走失败
 * 引导（复用桌面 updaterUi 口径，评审 R2 Important 同款防误报）。
 */
export function confirmViewFor(
  outcome: InstallOutcome | string,
  fallbackVersion = "",
): ConfirmView {
  if (typeof outcome === "string") {
    if (isInstallInProgressError(outcome)) {
      return { kind: "install-in-progress", text: installInProgressText() };
    }
    if (isNoNewerVersionError(outcome)) {
      return { kind: "no-newer", text: outcome };
    }
    return { kind: "failed", text: installFailedText(fallbackVersion, outcome), retryable: true };
  }
  return outcome.status === "install_started"
    ? { kind: "waiting-restart", version: outcome.version }
    : { kind: "failed", text: installFailedText(outcome.version, outcome.message), retryable: true };
}

/** 红点详情 → 横幅数据：null = 不出横幅。available=false / 版本缺失 / 响应为
 *  null（查询失败时组件折为 null）一律按无新版（提示副产物，宁可漏报不误报）。 */
export function bannerForDetail(
  detail: UpdateBadgeDetail | null | undefined,
): { version: string; notes: string | null } | null {
  if (
    typeof detail !== "object" ||
    detail === null ||
    detail.available !== true ||
    typeof detail.version !== "string" ||
    detail.version === ""
  ) {
    return null;
  }
  return { version: detail.version, notes: typeof detail.notes === "string" ? detail.notes : null };
}

// ── 文案（中文文案集中在此，组件只渲染；成功口径 import 桌面 succeededText）──

/** 横幅标题（规格用户视角：「有新版本 vX → 点击升级」） */
export function webBannerText(version: string): string {
  return `有新版本 v${version}`;
}

export const WEB_BANNER_ACTION_TEXT = "点击升级";
export const WEB_CONFIRM_TEXT = "确认升级";

/** 下载等待态（不展示百分比，规格 D 状态机） */
export function waitingRestartText(): string {
  return "正在下载升级包，应用将自动重启，本页会短暂断开——恢复后自动刷新";
}

/** 重启占位（探活失败，服务端已退出） */
export function restartingText(): string {
  return "正在重启中…";
}

/** 出口2 兜底文案（不再转圈，页面可手动刷新；重试=手动刷新，不出按钮） */
export function restartTimeoutText(): string {
  return "升级可能未完成，请到电脑端确认，或稍后刷新本页";
}

// ── 重启等待：探活节奏 + 10 分钟兜底（出口2）──────────────────────────────

/** 服务端探活间隔：确认后轮询 health_check，失败即服务端已退出（切 restarting） */
export const SERVER_PROBE_INTERVAL_MS = 3_000;

/** 出口2 的兜底窗口（规格 F7 取值）：确认起 10 分钟没等到重启即转兜底出口 */
export const RESTART_WATCH_MS = 10 * 60 * 1000;

/** 确认时刻起是否已超兜底窗口（纯函数，时间入参可注入可测；边界含到点） */
export function isRestartWatchExpired(confirmedAtMs: number, nowMs: number): boolean {
  return nowMs - confirmedAtMs >= RESTART_WATCH_MS;
}

/**
 * SSE 帧 → 该不该整页刷新（出口4 的触发半边）。epoch = 安装 id + 进程启动
 * 毫秒（与 versionSync 同源口径）：与确认时刻的基线不同 = 服务端进程重启过 =
 * 新版本已在跑 → 整页刷新拿新 bundle。基线为 null（没见过任何帧，无从判定）
 * 不刷，交给 10 分钟兜底。
 */
export function shouldReloadForEpoch(baseline: string | null, frameEpoch: string): boolean {
  return baseline !== null && frameEpoch !== baseline;
}

// ── 出口4 的比对半边：刷新落地后 sessionStorage 自证 ──────────────────────

/** sessionStorage 键：确认升级时记下的目标版本（整页刷新后由新 bundle 比对清账） */
export const PENDING_UPGRADE_KEY = "antfeedinglog.webupdate.pendingVersion";

export type UpgradeReloadResult = "upgraded" | "stale" | "none";

/**
 * 刷新落地后的比对（纯函数）：stored = sessionStorage 里的目标版本
 * （null = 没记过）。upgraded：与当前 bundle 版本相等 → 弹「已升级到 vX」；
 * stale：旧版本回来了（不弹）；none：无记录。upgraded/stale 都由调用方清记录。
 */
export function upgradeResultForReload(
  stored: string | null,
  currentVersion: string,
): UpgradeReloadResult {
  if (stored === null) return "none";
  return stored === currentVersion ? "upgraded" : "stale";
}
