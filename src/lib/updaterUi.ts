/**
 * 更新 UI 纯逻辑（票 06）：设置页「检查更新」节的状态映射 / 进度节流 / 文案选择。
 * Rust 契约（票 02/05 落定，serde tag=status snake_case，见 src-tauri/src/updater.rs）：
 * - check_update_now → CheckOutcome：up_to_date / update_available{version,notes}；
 *   检查失败折为 Err（invoke reject 字符串，一次性展示）。
 * - confirm_and_install → InstallOutcome：install_started{version}（Windows 下进程
 *   随即退出）/ install_failed{version,message}（引导重试/手动下载）。
 * - get_update_state → UpdateState：idle / last_install_succeeded{version} /
 *   last_install_incomplete{version}（升级未完成引导）。
 * - 事件 update-download-progress 负载 DownloadProgress：逐 chunk 无节流，
 *   展示节流在本层（shouldRefreshProgress）。
 */

/** 下载进度事件负载（Rust updater::DownloadProgress；downloaded=累计字节） */
export interface DownloadProgress {
  downloaded: number;
  total: number | null;
}

/** check_update_now 的 Ok 侧（失败走 reject，见 checkViewForOutcome） */
export type CheckOutcome =
  | { status: "up_to_date" }
  | { status: "update_available"; version: string; notes: string | null };

/** confirm_and_install 的 Ok 侧 */
export type InstallOutcome =
  | { status: "install_started"; version: string }
  | { status: "install_failed"; version: string; message: string };

/** get_update_state 返回（启动判定/确认流失败路径暂存的三态） */
export type UpdateState =
  | { status: "idle" }
  | { status: "last_install_succeeded"; version: string }
  | { status: "last_install_incomplete"; version: string };

// ── 文案选择（中文文案集中在此，组件只渲染）────────────────────────────────

/** 无更新：平静一句话，不催促 */
export function upToDateText(): string {
  return "已是最新版本";
}

/** 有新版标题 */
export function updateAvailableTitle(version: string): string {
  return `发现新版本 v${version}`;
}

/** 升级成功提示（上次"想升"的版本 ≤ 当前版本） */
export function succeededText(version: string): string {
  return `已升级到 v${version}。`;
}

/** 升级未完成引导文案（含目标版本与两个出口） */
export function incompleteGuidance(version: string): string {
  return `上次升级未完成（目标 v${version}），可在下方重试，或手动下载安装包。`;
}

/** 安装失败文案：版本 + 原因（重试/手动下载按钮由组件给出口）；版本未知时不留悬空的「v」 */
export function installFailedText(version: string, message: string): string {
  const prefix = version ? `升级到 v${version} 未完成` : "升级未完成";
  return `${prefix}：${message}`;
}

/** Rust lib.rs 防重入（CONFIRM_IN_FLIGHT）的固定拒绝串：安装其实已在后台健康
 * 进行（评审 R2 Important）——必须与真失败区分，否则误报失败诱导重试/手动下载。 */
const INSTALL_IN_PROGRESS_MARKER = "已有安装流程正在进行";

/** 识别防重入拒绝（在途安装误报防护） */
export function isInstallInProgressError(message: string): boolean {
  return message.includes(INSTALL_IN_PROGRESS_MARKER);
}

/** 「安装已在进行中」的平静提示：不出重试/手动下载（下载正在正常走，别劝退） */
export function installInProgressText(): string {
  return "安装正在进行中，请稍候。下载完成时应用将重启以完成安装。";
}

/** 安装已启动提示（进程将退出重启） */
export function installStartedText(): string {
  return "下载完成，应用将重启以完成安装。";
}

// ── 检查结果三态 → 展示视图 ────────────────────────────────────────────────

/** 检查结果的展示视图：idle（未查）/ up_to_date / available / failed（一次性） */
export type CheckView =
  | { kind: "idle" }
  | { kind: "up_to_date"; text: string }
  | { kind: "available"; version: string; notes: string | null }
  | { kind: "failed"; message: string };

/** Ok 侧成功两态映射；检查失败（invoke reject 的 string）由调用方包成 failed */
export function checkViewForOutcome(outcome: CheckOutcome | string): CheckView {
  if (typeof outcome === "string") {
    return { kind: "failed", message: outcome };
  }
  return outcome.status === "up_to_date"
    ? { kind: "up_to_date", text: upToDateText() }
    : { kind: "available", version: outcome.version, notes: outcome.notes };
}

// ── 安装结果两态 → 展示视图 ────────────────────────────────────────────────

/** 安装流的展示视图：进行中 / 已启动（将重启）/ 后台已在安装（平静等待）/ 失败 */
export type InstallView =
  | { kind: "installing" }
  | { kind: "started"; version: string; text: string }
  | { kind: "in_progress"; text: string }
  | { kind: "failed"; version: string; message: string; text: string };

/**
 * 安装结果 → 展示视图。字符串 = invoke 的 Err（再次检查失败/写标记失败/
 * Rust 防重入拒绝等）：防重入拒绝映射为「进行中」平静态（不走失败引导），
 * 其余折为失败态（version 用 fallbackVersion 兜底，让文案带上目标版本）。
 */
export function installViewForOutcome(
  outcome: InstallOutcome | string,
  fallbackVersion = "",
): InstallView {
  if (typeof outcome === "string") {
    if (isInstallInProgressError(outcome)) {
      return { kind: "in_progress", text: installInProgressText() };
    }
    return {
      kind: "failed",
      version: fallbackVersion,
      message: outcome,
      text: installFailedText(fallbackVersion, outcome),
    };
  }
  return outcome.status === "install_started"
    ? { kind: "started", version: outcome.version, text: installStartedText() }
    : {
        kind: "failed",
        version: outcome.version,
        message: outcome.message,
        text: installFailedText(outcome.version, outcome.message),
      };
}

// ── 启动残留（get_update_state）→ 引导横幅 ─────────────────────────────────

/** 状态横幅视图：none 不出；succeeded 成功提示；incomplete 未完成引导 */
export type StateBannerView =
  | { kind: "none" }
  | { kind: "succeeded"; text: string }
  | { kind: "incomplete"; version: string; text: string };

export function stateBannerFor(state: UpdateState): StateBannerView {
  switch (state.status) {
    case "idle":
      return { kind: "none" };
    case "last_install_succeeded":
      return { kind: "succeeded", text: succeededText(state.version) };
    case "last_install_incomplete":
      return {
        kind: "incomplete",
        version: state.version,
        text: incompleteGuidance(state.version),
      };
  }
}

// ── 下载进度：视图换算 + 展示节流（事件逐 chunk 无节流，展示自己做）─────────

/** 进度展示视图：percent 在 total 缺失/为 0 时为 null，封顶 100 */
export interface ProgressView {
  downloaded: number;
  total: number | null;
  percent: number | null;
}

export function toProgressView(p: DownloadProgress): ProgressView {
  const percent =
    p.total !== null && p.total > 0
      ? Math.min(100, Math.floor((p.downloaded / p.total) * 100))
      : null;
  return { downloaded: p.downloaded, total: p.total, percent };
}

/** 字节数人性化：B / KB / MB / GB（KB 起保留 1 位小数） */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v.toFixed(1)} ${units[i]}`;
}

/** 进度行文案：有总量带百分比（如「已下载 2.5 MB / 10.0 MB（25%）」） */
export function progressText(p: ProgressView): string {
  const done = `已下载 ${formatBytes(p.downloaded)}`;
  if (p.total === null || p.total <= 0) return done;
  const percent = p.percent === null ? "" : `（${p.percent}%）`;
  return `${done} / ${formatBytes(p.total)}${percent}`;
}

/** 展示节流的默认最小间隔（票面建议 ≥200ms） */
export const PROGRESS_REFRESH_INTERVAL_MS = 200;

/** 上次实际刷上屏的进度（组件保存，喂回 shouldRefreshProgress） */
export interface ShownProgress {
  atMs: number;
  percent: number | null;
}

/**
 * 该不该把这条进度刷上屏（节流裁判，时间入参可注入可测）：
 * 首次必刷；百分比变了 ≥1 个百分点立即刷；否则距上次刷新 ≥ minIntervalMs 才刷。
 * total 缺失（percent=null）退化为纯时间节流。
 */
export function shouldRefreshProgress(
  last: ShownProgress | null,
  incoming: ProgressView,
  nowMs: number,
  minIntervalMs: number = PROGRESS_REFRESH_INTERVAL_MS,
): boolean {
  if (last === null) return true;
  if (
    incoming.percent !== null &&
    last.percent !== null &&
    Math.abs(incoming.percent - last.percent) >= 1
  ) {
    return true;
  }
  return nowMs - last.atMs >= minIntervalMs;
}
