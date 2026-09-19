/**
 * 统一调用层（webui-checkin 票 01）：前端访问后端的唯一出入口。
 *
 * - 桌面（Tauri WebView，以 `window.__TAURI_INTERNALS__` 判定）：透传
 *   `@tauri-apps/api/core` 的 invoke / `@tauri-apps/api/event` 的 listen，
 *   命令名、入参对象、返回值与既有直调逐字一致（本票是纯预重构，行为零变化）。
 * - 浏览器（网页端）：对内嵌 HTTP 服务的约定端点 `POST /api/cmd` 发 JSON
 *   （体 `{ cmd, args }`），凭证以 `Authorization: Bearer <token>` 携带（读
 *   localStorage，键见 WEBUI_TOKEN_KEY；票 04 起由页面写入）。HTTP 服务的
 *   白名单端点随票 05 落地后本路径自然生效；事件订阅的浏览器侧（SSE）同样随
 *   票 05 接线，当前返回立即退订的空实现。
 *
 * 约定：
 * - 每个前端在用的 Tauri 命令一个类型化包装（cmdFn 构造，入参对象沿用各调用点
 *   现状键名，DTO 沿用 src/types.ts / lib/updaterUi.ts）；组件一律 import 包装，
 *   不得直接 import invoke/listen（验收标准）。
 * - 包装函数挂 `cmdName`（命令名）：排障可用，也是组件测试统一 mock 工厂
 *   （src/testing/ipcMock.ts）按命令名路由到唯一 invokeMock 的依据。
 * - Rust 侧 53 个命令中前端未调用的 4 个（health_check / set_action_policy /
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
  Colony,
  ColonyInput,
  FoodInput,
  FoodItem,
  LocationInput,
  LocationItem,
  LogFilter,
  LogPage,
  LogUpdateInput,
  PushoverStatus,
  RestoreApplyOutcome,
  RestoreSummary,
  StatsPayload,
  TestNotifyOutcome,
} from "../types";
import type { CheckOutcome, InstallOutcome, UpdateState } from "./updaterUi";

/** 网页端访问凭证的 localStorage 键（票 04 起由页面写入；为空时不带 Authorization 头）。 */
export const WEBUI_TOKEN_KEY = "antfeedinglog.webui.token";

/** 退订函数（与 Tauri listen 返回的 unlisten 同形）。 */
export type UnlistenFn = () => void;

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

/**
 * 事件订阅包装（组件不得直接 import listen）。桌面 = Tauri 事件，事件对象
 * 拆包为 payload 再交处理器（与原各监听点取 `event.payload` 等价）；浏览器 =
 * SSE，随票 05 接线，当前返回立即退订的空实现（订阅不报错也不泄漏）。
 */
export function subscribe<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (isTauri()) {
    return tauriListen<T>(event, (e) => handler(e.payload));
  }
  return Promise.resolve(() => {});
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

// ── 前端异常转发（main.ts 全局错误钩子）──

export const logFrontendError = cmdFn<{ message: string }, void>("log_frontend_error");
