<script setup lang="ts">
/**
 * 更新面板（release-update 票 06）：设置弹窗「更新」tab 的本体。
 * - 当前版本 + 「立即检查更新」按钮 → check_update_now 三态：无更新（平静
 *   「已是最新」）/ 有新版（版本号 + 说明 + 确认操作）/ 失败（一次性提示，不轰炸）。
 * - 有新版 → 「下载并安装」确认操作 → confirm_and_install（Rust 侧编排：再次检查
 *   → 写标记 → 下载 → 安装）；进度经 update-download-progress 事件展示，逐 chunk
 *   无节流事件在本组件用 shouldRefreshProgress 节流上屏；install_started 提示
 *   「将重启以完成安装」；install_failed 给「重试 / 手动下载」出口（下载超时等
 *   失败有路可走）。前置失败（Err，如再次检查无新版）同走失败出口。
 * - 挂载查 get_update_state：升级未完成残留 → 节顶引导（重试在下方检查流 /
 *   手动下载按钮直发 open_releases_page）；升级成功 → 平静的「已升级到 vX」。
 * - 前端不经 updater JS API，全部走 Tauri command（capabilities 无 updater 权限）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  checkViewForOutcome,
  installViewForOutcome,
  progressText,
  shouldRefreshProgress,
  stateBannerFor,
  toProgressView,
  updateAvailableTitle,
  type CheckOutcome,
  type CheckView,
  type InstallOutcome,
  type InstallView,
  type ProgressView,
  type ShownProgress,
  type StateBannerView,
  type UpdateState,
} from "../lib/updaterUi";

const currentVersion = ref("");
const banner = ref<StateBannerView>({ kind: "none" });
const checkView = ref<CheckView>({ kind: "idle" });
const checking = ref(false);
/** 安装流视图；null = 不在安装流（确认行可见的条件之一） */
const install = ref<InstallView | null>(null);
const installing = ref(false);
/** 已进入安装启动态（进程将退出重启）：窗口期内检查/确认都不再放行 */
const installStarted = computed(() => install.value?.kind === "started");
/** 用户点了「暂不更新」：收起确认行，再点「立即检查更新」会重新给出 */
const declined = ref(false);
/** 上屏的进度（节流后的展示值）；null = 尚未有事件（显示「准备下载…」） */
const progress = ref<ProgressView | null>(null);
let lastShown: ShownProgress | null = null;
let unlistenProgress: (() => void) | null = null;
/** 已卸载标志：listen 的 resolve 与卸载赛跑时，输了就立即退订（评审 R2 Minor-1） */
let disposed = false;
const releasesError = ref("");

const progressWidth = computed(() => {
  const percent = progress.value?.percent;
  return percent === null || percent === undefined ? "30%" : `${percent}%`;
});

const progressIndeterminate = computed(
  () => progress.value === null || progress.value.percent === null,
);

async function loadVersionAndState() {
  try {
    const [version, state] = await Promise.all([
      invoke<string>("get_app_version"),
      invoke<UpdateState>("get_update_state"),
    ]);
    currentVersion.value = version;
    banner.value = stateBannerFor(state);
  } catch (e) {
    // 版本/状态只是展示性信息，失败不出横幅（不打扰）
    console.error("读取版本/更新状态失败", e);
  }
}

function onProgressEvent(payload: unknown) {
  const view = toProgressView(payload as Parameters<typeof toProgressView>[0]);
  const now = Date.now();
  if (!shouldRefreshProgress(lastShown, view, now)) return;
  lastShown = { atMs: now, percent: view.percent };
  progress.value = view;
}

onMounted(async () => {
  const unlisten = await listen<{ downloaded: number; total: number | null }>(
    "update-download-progress",
    (event) => onProgressEvent(event.payload),
  );
  if (disposed) {
    // 挂载即卸载（listen 未 resolve 前组件已销毁）：立即退订，监听器不泄漏
    unlisten();
    return;
  }
  unlistenProgress = unlisten;
  await loadVersionAndState();
});

onUnmounted(() => {
  disposed = true;
  unlistenProgress?.();
  unlistenProgress = null;
});

async function checkNow() {
  checking.value = true;
  declined.value = false;
  try {
    const outcome = await invoke<CheckOutcome>("check_update_now").catch((e) => String(e));
    checkView.value = checkViewForOutcome(outcome);
  } finally {
    checking.value = false;
  }
}

function decline() {
  declined.value = true;
}

/** 确认安装 / 失败后重试（Rust 流程内部会再次检查，重复确认安全） */
async function installNow() {
  if (installing.value) return;
  installing.value = true;
  declined.value = false;
  progress.value = null;
  lastShown = null;
  install.value = { kind: "installing" };
  // 失败视图的版本兜底：优先当前检查到的新版，其次残留目标版本（Err 时
  // install_started/install_failed 载荷拿不到版本，靠它让文案带上目标）
  const fallbackVersion =
    checkView.value.kind === "available"
      ? checkView.value.version
      : banner.value.kind === "incomplete"
        ? banner.value.version
        : "";
  try {
    // Err（防重入拒绝/再次检查失败/写标记失败）折为字符串进映射层：
    // 防重入拒绝 = 安装健康进行中，映射为平静「进行中」态，不走失败引导
    //（评审 R2 Important：切 tab 回来重复确认时不得误报失败）
    const outcome = await invoke<InstallOutcome>("confirm_and_install").catch((e) => String(e));
    install.value = installViewForOutcome(outcome, fallbackVersion);
  } finally {
    installing.value = false;
  }
}

async function openReleases() {
  releasesError.value = "";
  try {
    await invoke("open_releases_page");
  } catch (e) {
    releasesError.value = String(e);
  }
}
</script>

<template>
  <div class="update-body">
    <!-- 启动残留引导（升级未完成 / 升级成功） -->
    <p v-if="banner.kind === 'incomplete'" class="update-banner update-banner-warn">
      <span class="banner-text">{{ banner.text }}</span>
      <button class="btn row-btn manual-dl-btn" type="button" title="在浏览器打开发布页下载安装包" @click="openReleases">
        手动下载
      </button>
    </p>
    <p v-else-if="banner.kind === 'succeeded'" class="update-banner update-banner-ok">{{ banner.text }}</p>

    <div class="update-row">
      <span class="current-version">当前版本：v{{ currentVersion || "…" }}</span>
      <button class="btn check-btn" type="button" :disabled="checking || installing || installStarted" @click="checkNow">
        {{ checking ? "检查中…" : "立即检查更新" }}
      </button>
    </div>

    <!-- 三态：无更新 / 有新版 / 失败 -->
    <p v-if="checkView.kind === 'up_to_date'" class="update-ok">{{ checkView.text }}</p>

    <div v-else-if="checkView.kind === 'available'" class="update-available">
      <p class="update-title">{{ updateAvailableTitle(checkView.version) }}</p>
      <p v-if="checkView.notes" class="update-notes">{{ checkView.notes }}</p>
      <div v-if="!declined && install === null" class="confirm-row">
        <button class="btn primary install-btn" type="button" :disabled="installing" @click="installNow">
          下载并安装
        </button>
        <button class="btn decline-btn" type="button" :disabled="installing" @click="decline">暂不更新</button>
      </div>
    </div>

    <p v-else-if="checkView.kind === 'failed'" class="form-error check-error">
      检查更新失败：{{ checkView.message }}
    </p>

    <!-- 安装流：进度 / 将重启 / 失败出口 -->
    <div v-if="install !== null" class="install-area">
      <template v-if="install.kind === 'installing'">
        <div class="progress-track" :class="{ indeterminate: progressIndeterminate }">
          <div class="progress-fill" :style="{ width: progressWidth }"></div>
        </div>
        <p class="progress-text">{{ progress ? progressText(progress) : "准备下载…" }}</p>
      </template>
      <p v-else-if="install.kind === 'started'" class="saved-hint install-started">{{ install.text }}</p>
      <p v-else-if="install.kind === 'in_progress'" class="update-ok install-in-progress">{{ install.text }}</p>
      <div v-else class="install-failed">
        <p class="form-error install-failed-text">{{ install.text }}</p>
        <div class="confirm-row">
          <button class="btn retry-btn" type="button" :disabled="installing" @click="installNow">重试</button>
          <button class="btn manual-dl-btn" type="button" title="在浏览器打开发布页下载安装包" @click="openReleases">
            手动下载
          </button>
        </div>
      </div>
    </div>

    <p v-if="releasesError" class="form-error">{{ releasesError }}</p>

    <p class="hint">
      每天会自动静默检查一次更新，检查失败不会打扰你。点「下载并安装」后会先自动备份数据库再下载，
      完成时应用会重启以完成安装；下载约 30 秒超时，失败可在这里重试或手动下载安装包。
    </p>
  </div>
</template>

<style scoped>
.update-body {
  font-size: 14px;
}

.update-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  border-radius: 10px;
  padding: 10px 12px;
  margin-bottom: 12px;
  font-size: 13px;
}

.update-banner-warn {
  background: var(--hib-soft);
  color: var(--hib);
}

.update-banner-ok {
  background: var(--tile);
  color: var(--ok, #2e7d32);
}

.update-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
}

.current-version {
  color: var(--muted);
}

.update-ok {
  font-size: 13px;
  color: var(--ok, #2e7d32);
}

.update-title {
  font-weight: 600;
  margin-bottom: 6px;
}

.update-notes {
  font-size: 13px;
  color: var(--muted);
  white-space: pre-wrap;
  margin-bottom: 10px;
}

.confirm-row {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.check-error {
  margin-top: 8px;
}

.install-area {
  margin-top: 12px;
}

.progress-track {
  height: 8px;
  background: var(--tile);
  border-radius: 999px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--accent);
  border-radius: 999px;
  transition: width 0.2s ease;
}

/* total 未知（远端没给 Content-Length）：来回滑动的占位条 */
.progress-track.indeterminate .progress-fill {
  animation: progress-slide 1.2s ease-in-out infinite;
}

@keyframes progress-slide {
  from {
    margin-left: -30%;
  }
  to {
    margin-left: 100%;
  }
}

.progress-text {
  margin-top: 6px;
  font-size: 12px;
  color: var(--muted);
}

.install-started {
  margin-top: 4px;
}

.hint {
  margin-top: 14px;
  font-size: 12px;
  color: var(--muted);
}
</style>
