<script setup lang="ts">
/**
 * 网页端升级横幅 + 确认面板（webui-update 票 02，ADR-0010）：浏览器模式专属，
 * 桌面端不渲染（App 外壳 v-if="!isTauri()" 挡 + 本组件挂载双保险）。
 * - 挂载调一次 get_update_badge_detail；available=true 才出「有新版本 vY →
 *   点击升级」；首查期间不渲染任何横幅（数据未到不闪）；× 关闭后本次会话
 *   （组件内会话态）不再出现，不落 localStorage/sessionStorage。
 * - 确认面板：当前 vX（package.json，不经命令）→ vY + 更新说明（notes=null
 *   省略说明区）+「确认升级」。
 * - 状态机与文案在 webUpdaterUi.ts；确认后 busy → waiting-restart，探活
 *   （health_check 轮询）失败切 restarting；SSE 帧出现新 epoch（服务端进程
 *   重启过）→ 整页刷新拿新 bundle（出口4 触发半边）；确认起 10 分钟没等到
 *   → restart-timeout（出口2）。刷新落地后的「已升级到 vX」由挂载时的
 *   sessionStorage 比对给出（出口4 比对半边）。
 * - 提示走全局 toast 模块：成功「已升级到 vX」（出口4）、失败红色轻提示带
 *   原因（出口1/出口3，规格 D）；防重入平静提示照桌面 UpdatePanel 先例走
 *   面板内文案不弹。红条是持久出口（带重试），轻提示是即时提醒。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  confirmAndInstallUpdate,
  getUpdateBadgeDetail,
  healthCheck,
  isTauri,
  subscribe,
  type UnlistenFn,
} from "../lib/ipc";
import { succeededText } from "../lib/updaterUi";
import { showSuccess, showError } from "../lib/toast";
import {
  bannerForDetail,
  confirmViewFor,
  isRestartWatchExpired,
  PENDING_UPGRADE_KEY,
  RESTART_WATCH_MS,
  restartingText,
  restartTimeoutText,
  SERVER_PROBE_INTERVAL_MS,
  shouldReloadForEpoch,
  upgradeResultForReload,
  waitingRestartText,
  WEB_BANNER_ACTION_TEXT,
  WEB_CONFIRM_TEXT,
  webBannerText,
  type ConfirmView,
  type UpdateBadgeDetail,
} from "../lib/webUpdaterUi";
import pkg from "../../package.json";

/** 当前 bundle 版本（与桌面同取 package.json 的既有模式） */
const appVersion = pkg.version;

type Phase =
  | "idle" // 无新版 / 已收起 / 查询失败
  | "banner" // 有新版待点
  | "panel" // 确认面板
  | "busy" // 确认中·命令在途
  | "waiting-restart" // install_started，等服务端重启
  | "restarting" // 探活失败，服务端已退出
  | "failed" // 红条 + 原因（是否带重试看 failRetryable）
  | "install-in-progress" // 防重入平静提示
  | "restart-timeout"; // 出口2 兜底

const phase = ref<Phase>("idle");
const loaded = ref(false);
const banner = ref<{ version: string; notes: string | null } | null>(null);
/** × 关闭后的本次会话态：组件实例活着就不再出（不落任何持久存储） */
const dismissed = ref(false);
/** failed 红条是否带重试（false = 复查无新版，出口3） */
const failRetryable = ref(true);
/** 红条/平静提示的文案（来自 confirmViewFor） */
const noticeText = ref("");
/** 确认流的目标版本（Err 文案兜底用） */
const targetVersion = ref("");

let confirmedAtMs = 0;
let probeTimer: ReturnType<typeof setInterval> | null = null;
let watchTimer: ReturnType<typeof setTimeout> | null = null;
let unlisten: UnlistenFn | null = null;
/** SSE 基线 epoch：最近见过的帧 epoch（重启判定基准，见 shouldReloadForEpoch） */
let sseBaseline: string | null = null;
let disposed = false;

const bannerVisible = computed(
  () =>
    banner.value !== null &&
    !dismissed.value &&
    phase.value !== "waiting-restart" &&
    phase.value !== "restarting" &&
    phase.value !== "restart-timeout",
);
const panelVisible = computed(() => phase.value === "panel" || phase.value === "busy");
const hasContent = computed(
  () =>
    bannerVisible.value ||
    panelVisible.value ||
    phase.value === "waiting-restart" ||
    phase.value === "restarting" ||
    phase.value === "failed" ||
    phase.value === "install-in-progress" ||
    phase.value === "restart-timeout",
);

// ── 出口4 比对半边：刷新落地后 sessionStorage 自证 ─────────────────────────

function settlePendingUpgrade(): void {
  let stored: string | null = null;
  try {
    stored = sessionStorage.getItem(PENDING_UPGRADE_KEY);
  } catch {
    return; // 存储不可用：按无记录，什么都不弹
  }
  if (stored === null) return;
  sessionStorage.removeItem(PENDING_UPGRADE_KEY);
  // upgraded → 「已升级到 vX」；stale（旧版本回来了）→ 不弹；两者都已清记录
  if (upgradeResultForReload(stored, appVersion) === "upgraded") {
    showSuccess(succeededText(stored));
  }
}

// ── 红点详情与横幅 ─────────────────────────────────────────────────────────

async function loadBadge(): Promise<void> {
  let detail: UpdateBadgeDetail | null = null;
  try {
    detail = await getUpdateBadgeDetail();
  } catch (e) {
    // 首查失败静默：横幅是提示副产物，不打扰（下次进页面再查）
    console.error("读取升级红点详情失败", e);
  }
  loaded.value = true;
  // × 关过就不再复活（会话态）；出口3 的重读红点同样被这里兜住
  if (!dismissed.value) {
    banner.value = bannerForDetail(detail);
    if (banner.value !== null && phase.value === "idle") {
      phase.value = "banner";
    }
  }
}

/** 会话内重查红点（出口3 的「重读红点」；也是测试钩子） */
function recheckBadge(): Promise<void> {
  return loadBadge();
}

function openPanel(): void {
  if (banner.value !== null && phase.value === "banner") {
    phase.value = "panel";
  }
}

function closePanel(): void {
  if (phase.value === "panel") {
    phase.value = "banner";
  }
}

function dismissBanner(): void {
  dismissed.value = true;
  banner.value = null;
  if (phase.value === "banner" || phase.value === "panel") {
    phase.value = "idle";
  }
}

// ── 确认流（出口1/3 + 防重入）──────────────────────────────────────────────

async function confirmUpgrade(): Promise<void> {
  if (phase.value === "busy") return; // 防连点：命令在途不再发
  if (banner.value !== null) targetVersion.value = banner.value.version;
  phase.value = "busy";
  confirmedAtMs = Date.now();
  let outcome: Awaited<ReturnType<typeof confirmAndInstallUpdate>> | string;
  try {
    outcome = await confirmAndInstallUpdate();
  } catch (e) {
    outcome = String(e); // Err（检查失败类/防重入）与桌面同形：字符串
  }
  applyConfirmView(confirmViewFor(outcome, targetVersion.value));
}

function applyConfirmView(view: ConfirmView): void {
  switch (view.kind) {
    case "waiting-restart": {
      targetVersion.value = view.version;
      try {
        sessionStorage.setItem(PENDING_UPGRADE_KEY, view.version);
      } catch {
        // 存储不可用：出口4 的成功自证会缺席，重启等待路径照走
      }
      phase.value = "waiting-restart";
      startRestartWatch();
      return;
    }
    case "failed":
      failRetryable.value = view.retryable;
      noticeText.value = view.text;
      phase.value = "failed";
      // 规格 D 出口1：红色轻提示带原因（面板内红条是持久出口，轻提示是即时提醒；
      // 全局 toast 模块唯一出口）
      showError(view.text);
      return;
    case "install-in-progress":
      noticeText.value = view.text;
      phase.value = "install-in-progress";
      return;
    case "no-newer":
      // 出口3：红条带该文案 + 红色轻提示带原因、不弹重试；收横幅；重读红点
      //（服务端复查已对齐红点落库，前端只刷新视图；dismissed 兜底横幅不复活）
      failRetryable.value = false;
      noticeText.value = view.text;
      dismissed.value = true;
      banner.value = null;
      phase.value = "failed";
      showError(view.text);
      void recheckBadge();
      return;
  }
}

/** 失败红条的重试：重发确认（Rust 流程内部会再次检查，重复确认安全） */
function retryConfirm(): void {
  void confirmUpgrade();
}

// ── 重启等待：探活 + 新 epoch 刷新 + 10 分钟兜底（出口2/4 触发半边）────────

function startRestartWatch(): void {
  stopRestartWatch();
  probeTimer = setInterval(() => void probeServer(), SERVER_PROBE_INTERVAL_MS);
  watchTimer = setTimeout(() => {
    watchTimer = null;
    if (disposed) return;
    if (isRestartWatchExpired(confirmedAtMs, Date.now())) {
      stopRestartWatch();
      phase.value = "restart-timeout";
    }
  }, RESTART_WATCH_MS);
}

function stopRestartWatch(): void {
  if (probeTimer !== null) {
    clearInterval(probeTimer);
    probeTimer = null;
  }
  if (watchTimer !== null) {
    clearTimeout(watchTimer);
    watchTimer = null;
  }
}

/** 探活：resolve = 服务端还在（维持现态）；reject = 服务端已退出 → restarting */
async function probeServer(): Promise<void> {
  if (disposed || (phase.value !== "waiting-restart" && phase.value !== "restarting")) return;
  try {
    await healthCheck();
    // 探活恢复但还没见到新 epoch 帧：维持 restarting，帧一到就刷新
  } catch {
    if (phase.value === "waiting-restart") {
      phase.value = "restarting";
    }
  }
}

function onDataVersionFrame(payload: unknown): void {
  if (disposed) return;
  const epoch =
    typeof payload === "object" && payload !== null && typeof (payload as { epoch?: unknown }).epoch === "string"
      ? (payload as { epoch: string }).epoch
      : null;
  if (epoch === null) return;
  const waiting = phase.value === "waiting-restart" || phase.value === "restarting";
  if (waiting && shouldReloadForEpoch(sseBaseline, epoch)) {
    // 服务端进程重启过（新 epoch）：整页刷新拿新 bundle（出口4 触发半边）
    window.location.reload();
    return;
  }
  sseBaseline = epoch; // 基线只在不触发刷新时前移（要刷新的帧说明旧基线已过期作废）
}

// ── 生命周期 ───────────────────────────────────────────────────────────────

onMounted(() => {
  if (isTauri()) return; // 桌面端永不渲染这套 UI（双保险，外壳已挡）
  settlePendingUpgrade();
  void subscribe("data-version", onDataVersionFrame).then((un) => {
    if (disposed) {
      un();
      return;
    }
    unlisten = un;
  });
  void loadBadge();
});

onUnmounted(() => {
  disposed = true;
  stopRestartWatch();
  unlisten?.();
  unlisten = null;
});

defineExpose({ recheckBadge });
</script>

<template>
  <div v-if="hasContent" class="web-update">
    <!-- 横幅条：有新版待点（× 关闭本次会话不再出现） -->
    <div v-if="bannerVisible && banner" class="wu-banner">
      <span class="wu-banner-text">{{ webBannerText(banner.version) }}</span>
      <button class="wu-open-btn" type="button" @click="openPanel">{{ WEB_BANNER_ACTION_TEXT }}</button>
      <button class="wu-close-btn" type="button" title="本次会话不再提示" aria-label="关闭" @click="dismissBanner">×</button>
    </div>

    <!-- 确认面板：当前 vX → vY + 更新说明（notes=null 省略） -->
    <div v-if="panelVisible && banner" class="wu-panel">
      <p class="wu-panel-line">
        <span class="wu-current">当前 v{{ appVersion }}</span>
        <span class="wu-arrow" aria-hidden="true">→</span>
        <span class="wu-target">新版本 v{{ banner.version }}</span>
      </p>
      <p v-if="banner.notes" class="wu-notes">{{ banner.notes }}</p>
      <div class="wu-panel-actions">
        <button
          class="wu-confirm-btn"
          type="button"
          :disabled="phase === 'busy'"
          @click="confirmUpgrade"
        >
          {{ phase === "busy" ? "确认中…" : WEB_CONFIRM_TEXT }}
        </button>
        <button class="wu-cancel-btn" type="button" :disabled="phase === 'busy'" @click="closePanel">
          取消
        </button>
      </div>
    </div>

    <!-- 等待/重启占位（不展示百分比） -->
    <p v-if="phase === 'waiting-restart'" class="wu-strip wu-waiting">{{ waitingRestartText() }}</p>
    <p v-else-if="phase === 'restarting'" class="wu-strip wu-waiting">{{ restartingText() }}</p>

    <!-- 失败红条（带原因；出口3 不弹重试） -->
    <div v-else-if="phase === 'failed'" class="wu-strip wu-failed">
      <span class="wu-failed-text">{{ noticeText }}</span>
      <button v-if="failRetryable" class="wu-retry-btn" type="button" @click="retryConfirm">重试</button>
    </div>

    <!-- 防重入平静提示（不出重试：安装正在正常走，别劝退） -->
    <p v-else-if="phase === 'install-in-progress'" class="wu-strip wu-calm">{{ noticeText }}</p>

    <!-- 出口2 兜底：不再转圈，页面可手动刷新 -->
    <p v-else-if="phase === 'restart-timeout'" class="wu-strip wu-timeout">{{ restartTimeoutText() }}</p>
  </div>
</template>

<style scoped>
/* 顶栏下外壳层横幅：与顶栏同宽对齐（视觉变量取 App 根的 CSS 自定义属性） */
.web-update {
  padding: 10px 20px 0;
}

.wu-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  background: var(--accent-soft, #fdf1de);
  border: 1px solid var(--border-strong, #d8d1c4);
  border-radius: 10px;
  padding: 8px 12px;
  font-size: 13px;
}

.wu-banner-text {
  color: var(--accent-deep, #b45309);
  font-weight: 600;
}

.wu-open-btn,
.wu-confirm-btn,
.wu-retry-btn {
  padding: 4px 12px;
  border: 1px solid var(--accent, #d97706);
  border-radius: 8px;
  background: var(--card, #ffffff);
  color: var(--accent-deep, #b45309);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.wu-open-btn:hover,
.wu-confirm-btn:hover,
.wu-retry-btn:hover {
  background: var(--accent-soft, #fdf1de);
}

.wu-close-btn {
  margin-left: auto;
  border: none;
  background: transparent;
  color: var(--muted, #8f887d);
  font-size: 15px;
  line-height: 1;
  cursor: pointer;
  padding: 2px 4px;
}

.wu-close-btn:hover {
  color: var(--text, #2c2822);
}

.wu-panel {
  margin-top: 8px;
  background: var(--card, #ffffff);
  border: 1px solid var(--border-strong, #d8d1c4);
  border-radius: 10px;
  padding: 10px 12px;
  font-size: 13px;
  box-shadow: var(--shadow, 0 1px 2px rgba(60, 50, 30, 0.05));
}

.wu-panel-line {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
}

.wu-current {
  color: var(--muted, #8f887d);
}

.wu-arrow {
  color: var(--muted, #8f887d);
}

.wu-target {
  color: var(--accent-deep, #b45309);
}

.wu-notes {
  margin-top: 6px;
  color: var(--muted, #8f887d);
  white-space: pre-wrap;
}

.wu-panel-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.wu-cancel-btn {
  padding: 4px 12px;
  border: 1px solid var(--border-strong, #d8d1c4);
  border-radius: 8px;
  background: var(--card, #ffffff);
  color: var(--text, #2c2822);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.wu-strip {
  margin-top: 0;
  border-radius: 10px;
  padding: 8px 12px;
  font-size: 13px;
}

.wu-waiting {
  background: var(--accent-soft, #fdf1de);
  color: var(--accent-deep, #b45309);
}

.wu-failed,
.wu-timeout {
  display: flex;
  align-items: center;
  gap: 10px;
  background: var(--bad-soft, #fcebeb);
  color: var(--bad, #d13d3d);
}

.wu-calm {
  background: var(--tile, #faf8f5);
  color: var(--muted, #8f887d);
}

.wu-retry-btn {
  flex: none;
  margin-left: auto;
}

.wu-confirm-btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}
</style>
