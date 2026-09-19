/**
 * 统一调用层（webui-checkin 票 01）：前端访问后端的唯一出入口。
 *
 * - 桌面（Tauri WebView，以 `window.__TAURI_INTERNALS__` 判定）：透传
 *   `@tauri-apps/api/core` 的 invoke / `@tauri-apps/api/event` 的 listen，
 *   命令名、入参对象、返回值与既有直调逐字一致（本票是纯预重构，行为零变化）。
 * - 浏览器（网页端）：对内嵌 HTTP 服务的约定端点 `POST /api/cmd` 发 JSON
 *   （体 `{ cmd, args }`），凭证以 `Authorization: Bearer <token>` 携带（读
 *   localStorage，键见 WEBUI_TOKEN_KEY；票 04 起由页面写入）。HTTP 服务的
 *   白名单端点随票 05 落地后本路径自然生效；事件订阅的浏览器侧（SSE）随
 *   票 06 接线——data-version 事件走一次性票据 + EventSource，见本文件
 *   「浏览器 SSE」段。
 *
 * 约定：
 * - 每个前端在用的 Tauri 命令一个类型化包装（cmdFn 构造，入参对象沿用各调用点
 *   现状键名，DTO 沿用 src/types.ts / lib/updaterUi.ts）；组件一律 import 包装，
 *   不得直接 import invoke/listen（验收标准）。
 * - 包装函数挂 `cmdName`（命令名）：排障可用，也是组件测试统一 mock 工厂
 *   （src/testing/ipcMock.ts）按命令名路由到唯一 invokeMock 的依据。
 * - Rust 侧 58 个命令中前端未调用的 4 个（health_check / set_action_policy /
 *   list_hibernations / get_backup_status）不在此预置包装，随用随加。
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type {
  AbnormalExitInfo,
  ActionInput,
  AppSettings,
  BackupConfigInfo,
  BackupConfigInput,
  CareActionItem,
  CareLogInput,
  CheckinDigest,
  CheckinInput,
  CheckinUpdateInput,
  Colony,
  ColonyInput,
  FoodInput,
  FoodItem,
  LocationInput,
  LocationItem,
  LogFilter,
  LogPage,
  LogUpdateInput,
  NestCheckin,
  PushoverStatus,
  RestoreApplyOutcome,
  RestoreSummary,
  StatsPayload,
  TestNotifyOutcome,
  NetworkSegment,
  WebUiConfigInfo,
  WebUiSaveInput,
  WebUiSaveOutcome,
} from "../types";
import type { CheckOutcome, InstallOutcome, UpdateState } from "./updaterUi";

/** 网页端访问凭证的 localStorage 键（票 04 起由页面写入；为空时不带 Authorization 头）。 */
export const WEBUI_TOKEN_KEY = "antfeedinglog.webui.token";

/** 退订函数（与 Tauri listen 返回的 unlisten 同形）。 */
export type UnlistenFn = () => void;

/** SSE 版本帧（票 06：`/api/sse` 的 data 行 JSON；hello/version 两类）。 */
export interface SseVersionFrame {
  type: "hello" | "version";
  epoch: string;
  version: number;
}

/** 桌面 Tauri 环境判定：WebView 里由 Tauri 注入 `__TAURI_INTERNALS__`；纯浏览器没有。 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** 双模式路由的唯一决策点：桌面透传 Tauri invoke（无参命令不传第二参，与现状逐字一致），浏览器走 HTTP。 */
function call<R>(command: string, args?: Record<string, unknown>): Promise<R> {
  if (isTauri()) {
    return args === undefined ? tauriInvoke<R>(command) : tauriInvoke<R>(command, args);
  }
  return httpInvoke<R>(command, args);
}

/**
 * 浏览器路径：`POST /api/cmd`，JSON 体 `{ cmd, args }`（无参命令 args 为空对象）。
 * 返回体即命令返回值（空体归一为 null）；非 2xx 以响应体 `{ error }` 的字符串
 * reject（不可解析时 `HTTP <status>`）——与桌面 invoke 以字符串 reject 同形，
 * 组件层既有的 `String(e)` 错误展示逻辑不用区分环境。
 */
async function httpInvoke<R>(command: string, args?: Record<string, unknown>): Promise<R> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  const token = localStorage.getItem(WEBUI_TOKEN_KEY);
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  const res = await fetch("/api/cmd", {
    method: "POST",
    headers,
    body: JSON.stringify({ cmd: command, args: args ?? {} }),
  });
  const text = await res.text();
  let body: unknown = null;
  if (text !== "") {
    try {
      body = JSON.parse(text) as unknown;
    } catch {
      // 非 2xx：交给下面的统一错误路径（HTTP <status>）；
      // 2xx 且非空体却解析不了（网关劫持/代理注页）：明确报错，绝不静默归
      // null 让上层把「响应丢了」当「查询结果为空」用（票 05 评审 Minor）。
      if (res.ok) {
        throw `HTTP ${res.status} 不可解析响应`;
      }
      body = null;
    }
  }
  if (!res.ok) {
    const errText =
      typeof body === "object" && body !== null && "error" in body
        ? (body as { error: unknown }).error
        : null;
    throw typeof errText === "string" ? errText : `HTTP ${res.status}`;
  }
  return body as R;
}

// ── 浏览器 SSE：data-version 事件源（票 06，规格 B「SSE 短时票据」）─────────
//
// EventSource 建连带不了 Authorization 头，流程：先 POST /api/sse-ticket（主
// 凭证换 60 秒一次性票据）→ 连 `/api/sse?ticket=`。**票据建连即消费（一次性）
// ——所以必须在 onerror 里 close() 关掉 EventSource 原生自动重连**（原生重连
// 带着旧票只会 401 死循环），然后重取票据重建连，带 1s/3s/10s 封顶退避。
// 同一页面多个订阅者共享一条 SSE 连接（按 handler 计数管理生命周期）。
// 对账（hello/version 帧 → 是否重拉）在 versionSync.ts，本层只管连接与派发。

/** 重连退避序列（毫秒），越挫越狠、10 秒封顶。 */
const SSE_BACKOFF_MS = [1_000, 3_000, 10_000] as const;

interface BrowserSseState {
  es: EventSource | null;
  /** 取票进行中（防并发重复建连）。 */
  connecting: boolean;
  /** 连续失败计数（退避档位；onopen 成功即归零）。 */
  attempt: number;
  timer: ReturnType<typeof setTimeout> | null;
  handlers: Set<(payload: unknown) => void>;
}

let browserSse: BrowserSseState | null = null;

/** 把一帧派发给所有 data-version 订阅者（handler 异常互不牵连）。 */
function sseDispatch(frame: SseVersionFrame): void {
  const state = browserSse;
  if (!state) return;
  for (const handler of [...state.handlers]) {
    try {
      handler(frame);
    } catch (e) {
      console.error("[ipc] data-version 处理器异常", e);
    }
  }
}

/** 全部退订：断连接、撤销挂起的重连。 */
function sseTeardown(): void {
  const state = browserSse;
  browserSse = null;
  if (!state) return;
  if (state.timer !== null) clearTimeout(state.timer);
  state.es?.close();
}

/** 测试专用：复位浏览器 SSE 单例（模块级连接状态不跨测试泄漏）。 */
export function resetBrowserSseForTests(): void {
  sseTeardown();
}

/** 主凭证换一次性票据（失败 throw，调用方按退避重试）。 */
async function fetchSseTicket(): Promise<string> {
  const headers: Record<string, string> = {};
  const token = localStorage.getItem(WEBUI_TOKEN_KEY);
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  const res = await fetch("/api/sse-ticket", { method: "POST", headers });
  if (!res.ok) {
    throw `HTTP ${res.status}`;
  }
  const body: unknown = await res.json();
  const ticket =
    typeof body === "object" && body !== null
      ? (body as { ticket?: unknown }).ticket
      : undefined;
  if (typeof ticket !== "string" || ticket === "") {
    throw "票据响应缺 ticket";
  }
  return ticket;
}

function scheduleSseReconnect(): void {
  const state = browserSse;
  if (!state || state.timer !== null || state.handlers.size === 0) return;
  const delay = SSE_BACKOFF_MS[Math.min(state.attempt, SSE_BACKOFF_MS.length - 1)];
  state.attempt += 1;
  state.timer = setTimeout(() => {
    if (browserSse) {
      browserSse.timer = null;
      sseConnect();
    }
  }, delay);
}

function sseConnect(): void {
  const state = browserSse;
  if (!state || state.es || state.connecting) return;
  state.connecting = true;
  fetchSseTicket()
    .then((ticket) => {
      const cur = browserSse;
      if (!cur) return; // 取票期间已全部退订
      cur.connecting = false;
      const es = new EventSource(`/api/sse?ticket=${encodeURIComponent(ticket)}`);
      cur.es = es;
      es.onopen = () => {
        cur.attempt = 0; // 连上过：退避归零
      };
      es.onmessage = (ev) => {
        try {
          const frame = JSON.parse(String(ev.data)) as SseVersionFrame;
          if (
            frame !== null &&
            typeof frame === "object" &&
            (frame.type === "hello" || frame.type === "version") &&
            typeof frame.epoch === "string" &&
            typeof frame.version === "number"
          ) {
            sseDispatch(frame);
          }
          // 形状不对的帧：忽略，等下一帧
        } catch {
          // 半行/非 JSON：忽略
        }
      };
      es.onerror = () => {
        // 票据一次性：原生自动重连只会拿旧票撞 401 —— close 关掉它，重取票据再来
        es.close();
        if (browserSse === cur) {
          cur.es = null;
          scheduleSseReconnect();
        }
      };
    })
    .catch(() => {
      const cur = browserSse;
      if (!cur) return;
      cur.connecting = false;
      scheduleSseReconnect();
    });
}

/**
 * 事件订阅包装（组件不得直接 import listen）。桌面 = Tauri 事件，事件对象
 * 拆包为 payload 再交处理器（与原各监听点取 `event.payload` 等价）；浏览器 =
 * SSE（票 06）：仅 `data-version` 有源——首帧 hello、此后每次版本 bump 一帧
 * version，负载 `{type, epoch, version}` 原样透传。恢复完成没有独立事件：服务
 * 端恢复后把版本抬到已广播最大值+1 再广播 version 帧，订阅方（versionSync）
 * 对账必判落后 → 刷新。其余事件桌面专属，返回立即退订的空实现。
 */
export function subscribe<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (isTauri()) {
    return tauriListen<T>(event, (e) => handler(e.payload));
  }
  if (event !== "data-version") {
    return Promise.resolve(() => {});
  }
  const state = (browserSse ??= {
    es: null,
    connecting: false,
    attempt: 0,
    timer: null,
    handlers: new Set(),
  });
  state.handlers.add(handler as (payload: unknown) => void);
  if (!state.es && state.timer === null) {
    sseConnect();
  }
  return Promise.resolve(() => {
    state.handlers.delete(handler as (payload: unknown) => void);
    if (state.handlers.size === 0) {
      sseTeardown();
    }
  });
}

/** 命令包装函数：入参对象类型 A、返回类型 R、命令名挂 cmdName。 */
export interface CommandFn<A, R> {
  (args: A): Promise<R>;
  readonly cmdName: string;
}

/** 命令包装统一构造器：A = void 表示无入参命令（调用方零参调用）。 */
function cmdFn<A, R>(command: string): CommandFn<A, R> {
  const fn = (args?: A): Promise<R> =>
    args === undefined ? call<R>(command) : call<R>(command, args as unknown as Record<string, unknown>);
  return Object.assign(fn, { cmdName: command });
}

// ── 首页（窝卡片墙）──

export const listColonies = cmdFn<void, Colony[]>("list_colonies");
export const listLocations = cmdFn<void, LocationItem[]>("list_locations");

// ── 窝的新建/编辑（ColonyFormDialog）──

export const createColony = cmdFn<{ input: ColonyInput }, void>("create_colony");
export const updateColony = cmdFn<{ id: number; input: ColonyInput }, void>("update_colony");
export const archiveColony = cmdFn<{ id: number }, void>("archive_colony");
export const deleteColony = cmdFn<{ id: number }, void>("delete_colony");

// ── 记账（ColonyCard → FeedDialog / QuickLogDialog）──

export const listFoods = cmdFn<void, FoodItem[]>("list_foods");
export const logCare = cmdFn<{ input: CareLogInput }, void>("log_care");

// ── 巢况登记（webui-checkin 票 02；时间线弹窗 NestCheckinDialog）──

export const saveCheckin = cmdFn<{ input: CheckinInput }, NestCheckin>("save_checkin");
export const listCheckins = cmdFn<{ colonyId: number }, NestCheckin[]>("list_checkins");
export const updateCheckin = cmdFn<
  { id: number; input: CheckinUpdateInput },
  NestCheckin
>("update_checkin");
export const deleteCheckin = cmdFn<{ id: number }, void>("delete_checkin");
export const getCheckinDigest = cmdFn<{ colonyId: number }, CheckinDigest>("get_checkin_digest");

// ── 冬眠（HibernationDialog；入参键沿用现状 camelCase，Tauri 侧自行映射）──

export const startHibernation = cmdFn<
  { colonyId: number; startDate: string; expectedEndDate: string },
  void
>("start_hibernation");
export const confirmWake = cmdFn<{ colonyId: number; actualEndDate: string }, void>("confirm_wake");
export const updateExpectedEnd = cmdFn<
  { colonyId: number; newExpectedEndDate: string },
  void
>("update_expected_end");
export const addPastHibernation = cmdFn<
  { colonyId: number; startDate: string; endDate: string },
  void
>("add_past_hibernation");

// ── 地点管理（LocationManagerPanel）──

export const saveLocation = cmdFn<{ input: LocationInput }, void>("save_location");
export const setLocationEnabled = cmdFn<{ id: number; enabled: boolean }, void>("set_location_enabled");
export const eraseLocation = cmdFn<{ id: number }, void>("erase_location");

// ── 记录列表页（LogListPage）──

export const listActions = cmdFn<void, CareActionItem[]>("list_actions");
export const listLogs = cmdFn<{ filter: LogFilter }, LogPage>("list_logs");
export const updateLog = cmdFn<{ id: number; input: LogUpdateInput }, void>("update_log");
export const deleteLog = cmdFn<{ id: number }, void>("delete_log");

// ── 统计页（StatsPage）──

export const earliestLogDate = cmdFn<void, string | null>("earliest_log_date");
export const getStats = cmdFn<
  { colonyId: number | null; startDate: string; endDate: string },
  StatsPayload
>("get_stats");

// ── 设置弹窗：字典（SettingsDialog）──

export const saveAction = cmdFn<{ input: ActionInput }, void>("save_action");
export const setActionEnabled = cmdFn<{ id: number; enabled: boolean }, void>("set_action_enabled");
export const eraseAction = cmdFn<{ id: number }, void>("erase_action");
export const saveFood = cmdFn<{ input: FoodInput }, void>("save_food");
export const setFoodEnabled = cmdFn<{ id: number; enabled: boolean }, void>("set_food_enabled");
export const eraseFood = cmdFn<{ id: number }, void>("erase_food");

// ── 设置弹窗：通知（票 06）──

export const getSettings = cmdFn<void, AppSettings>("get_settings");
export const setSettings = cmdFn<{ input: AppSettings }, AppSettings>("set_settings");
export const pushoverStatus = cmdFn<void, PushoverStatus>("pushover_status");
export const sendTestNotification = cmdFn<void, TestNotifyOutcome>("send_test_notification");

// ── 设置弹窗：数据 / 日志 / 备份 / 恢复（票 09 与数据安全二期）──

export const revealDataFolder = cmdFn<void, string>("reveal_data_folder");
export const openLogsFolder = cmdFn<void, string>("open_logs_folder");
export const getRecentErrors = cmdFn<void, string[]>("get_recent_errors");
export const getLastAbnormalExit = cmdFn<void, AbnormalExitInfo | null>("get_last_abnormal_exit");
export const getBackupConfig = cmdFn<void, BackupConfigInfo>("get_backup_config");
export const setBackupConfig = cmdFn<{ input: BackupConfigInput }, BackupConfigInfo>("set_backup_config");
export const pickBackupDir = cmdFn<void, string | null>("pick_backup_dir");
export const backupTo = cmdFn<void, string | null>("backup_to");
export const pickRestoreFile = cmdFn<{ defaultDir: string | null }, string | null>("pick_restore_file");
export const restorePreview = cmdFn<{ path: string }, RestoreSummary>("restore_preview");
export const restoreApply = cmdFn<{ path: string }, RestoreApplyOutcome>("restore_apply");
export const exportData = cmdFn<{ format: "csv" | "json" }, string | null>("export_data");

// ── 更新（UpdatePanel；事件 update-download-progress 走 subscribe）──

export const getAppVersion = cmdFn<void, string>("get_app_version");
export const getUpdateState = cmdFn<void, UpdateState>("get_update_state");
export const checkUpdateNow = cmdFn<void, CheckOutcome>("check_update_now");
export const confirmAndInstall = cmdFn<void, InstallOutcome>("confirm_and_install");
export const openReleasesPage = cmdFn<void, void>("open_releases_page");

// ── 网页端设置（webui-checkin 票 03；防火墙 UAC 路径真机冒烟，不在前端测）──

export const listNetworkSegments = cmdFn<void, NetworkSegment[]>("list_network_segments");
export const getWebUiConfig = cmdFn<void, WebUiConfigInfo>("get_webui_config");
export const saveWebUiConfig = cmdFn<{ input: WebUiSaveInput }, WebUiSaveOutcome>(
  "save_webui_config",
);
export const regenerateWebUiToken = cmdFn<void, WebUiConfigInfo>("regenerate_token");
export const getWebUiAccessUrl = cmdFn<void, string>("get_access_url");
export const getWebUiWizardDone = cmdFn<void, boolean>("get_webui_wizard_done");
export const markWebUiWizardDone = cmdFn<void, void>("mark_webui_wizard_done");

// ── 前端异常转发（main.ts 全局错误钩子）──

export const logFrontendError = cmdFn<{ message: string }, void>("log_frontend_error");
