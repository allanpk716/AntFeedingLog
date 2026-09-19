<script setup lang="ts">
/**
 * 设置弹窗（票 04 字典管理 + 票 06 通知 + 票 09 数据收口 + release-update 票 06 更新）：
 * 操作 / 食物 / 地点 / 通知 / 数据 / 更新 六个 tab。
 * - 操作行可编辑：名字、性质（提醒/仅登记）、建议间隔（提醒类显示）、
 *   「喂食」标记（带提示，允许编辑不强制唯一）、停用/启用、删除
 *   （预置项或被历史记录/提醒台账引用时删除禁用，只能停用——规则 10 + 反馈第二轮 F2）。
 * - 食物行：名字、建议间隔（F3：留空=只按喂食统一周期）、停用/启用、删除（预置或被引用禁用）。
 * - 地点 tab 复用 LocationManagerPanel。
 * - 通知 tab（票 06 + 反馈第二轮 F4）：推送通知总开关（桌面 + 手机，分类子开关作废）+
 *   临近出眠提前天数 + Pushover 配置状态 + 「发送测试通知」按钮（双通道分别回显结果，
 *   排障用）；开机自启开关（票 09）随保存一起落库，
 *   Rust 侧 set_settings 同步自启插件状态。
 * - 数据 tab（票 09）：打开数据文件夹 / 安全备份（Rust 拷贝库文件，无需退出）/
 *   导出 CSV / JSON（归档带走）；帮助文案写明手动拷贝需先从托盘真实退出。
 *   数据 tab 自动备份区（数据安全二期票 02）：开关 / 备份目录（系统文件夹选择框）/
 *   保留份数（1–365 校验）/ 上次备份状态（尚未备份/成功/失败+原因）；
 *   开关开着 ∧ 目录未设时显示「未生效」提示。配置存数据目录 backup-config.json
 *   （D1：库外文件，恢复整库不回滚备份设置）。
 *   恢复区（数据安全二期票 04）：「从备份恢复」按钮在自动备份区与手动按钮之间
 *   （D11）；流程 = 选文件（默认定位备份目录）→ 摘要预览（窝数/记录数/备份日期/
 *   备份内目录设置值 + 固定文案）→ 二段确认（与删除记录同待遇）→ 执行 →
 *   刷新或重启提示。整库替换语义（ADR-0002）。
 * - 更新 tab（release-update 票 06）：当前版本 / 立即检查更新 / 确认下载安装，
 *   全部走 Tauri command；升级未完成残留的引导也挂在本节顶（UpdatePanel）。
 *
 * 行级 停用/启用/删除 即时落库并抛 changed（外层刷新首页，卡片红/灰随之变化）；
 * 名字/排序/性质/间隔/喂食标记在本地行上积累，「保存」一次性按行序落库（sort=行下标），
 * 成功后重拉字典并抛 changed。停用项整行置灰。
 */
import { onMounted, ref } from "vue";
import {
  backupTo,
  eraseAction as eraseActionCmd,
  eraseFood as eraseFoodCmd,
  exportData as exportDataCmd,
  getBackupConfig,
  getLastAbnormalExit,
  getRecentErrors,
  getSettings,
  listActions,
  listFoods,
  listLocations,
  openLogsFolder,
  pickBackupDir as pickBackupDirCmd,
  pickRestoreFile,
  pushoverStatus as getPushoverStatus,
  restoreApply,
  restorePreview,
  revealDataFolder,
  saveAction,
  saveFood,
  sendTestNotification,
  setActionEnabled as setActionEnabledCmd,
  setBackupConfig,
  setFoodEnabled as setFoodEnabledCmd,
  setSettings,
} from "../lib/ipc";
import type {
  BackupConfigInput,
  BackupConfigInfo,
  LocationItem,
  PushoverStatus,
  RestoreSummary,
} from "../types";
import {
  buildActionRows,
  buildFoodRows,
  moveRow,
  toActionInputs,
  toFoodInputs,
  validateActionRows,
  validateFoodRows,
  type ActionRow,
  type FoodRow,
} from "../lib/dict";
import { DAYS_AHEAD_ERROR, toForm, toSettings, type NotifySettingsForm } from "../lib/notifySettings";
import { formatAbnormalExit } from "../lib/applog";
import {
  KEEP_COUNT_ERROR,
  autoBackupInactive,
  formatLastBackup,
  validateKeepCount,
} from "../lib/backupUi";
import {
  RESTORE_CONFIRM_TEXT,
  formatRestoreOutcome,
  summaryLooksSuspicious,
  summaryRows,
} from "../lib/restoreUi";
import LocationManagerPanel from "./LocationManagerPanel.vue";
import UpdatePanel from "./UpdatePanel.vue";

type Tab = "actions" | "foods" | "locations" | "notify" | "data" | "update";

const emit = defineEmits<{ close: []; changed: [] }>();

const activeTab = ref<Tab>("actions");
const actionRows = ref<ActionRow[]>([]);
const foodRows = ref<FoodRow[]>([]);
const locations = ref<LocationItem[]>([]);
const addActionName = ref("");
const addFoodName = ref("");
const error = ref("");
const busy = ref(false);

// ── 通知 tab（票 06 + 反馈第二轮 F4）──
const notifyForm = ref<NotifySettingsForm>({ master: true, daysAheadText: "7" });
const autostart = ref(true);
const notifyError = ref("");
const notifySaved = ref("");
const notifyBusy = ref(false);
const pushoverStatus = ref<PushoverStatus | null>(null);

async function load() {
  const [actions, foods, locs, s, pushStatus] = await Promise.all([
    listActions(),
    listFoods(),
    listLocations(),
    getSettings(),
    getPushoverStatus(),
  ]);
  actionRows.value = buildActionRows(actions);
  foodRows.value = buildFoodRows(foods);
  locations.value = locs;
  notifyForm.value = toForm(s);
  autostart.value = s.autostart_enabled;
  pushoverStatus.value = pushStatus;
}

onMounted(async () => {
  try {
    await load();
  } catch (e) {
    error.value = String(e);
  }
  // 日志区（票 01）加载失败静默：日志区是辅助信息，不值得为它报错打扰
  await loadLogSection();
  // 自动备份区（票 02）加载失败同样静默：配置读不出时整区隐藏，不挡其他功能区
  await loadBackupSection();
});

// ── 行级即时操作 ──

async function addActionRow(kind: "actions" | "foods") {
  const raw = (kind === "actions" ? addActionName : addFoodName).value.trim();
  if (!raw) {
    error.value = kind === "actions" ? "操作名字不能为空" : "食物名字不能为空";
    return;
  }
  const target: Array<{ id: number | null; name: string }> =
    kind === "actions" ? actionRows.value : foodRows.value;
  if (target.some((r) => r.name.trim() === raw)) {
    error.value = `名字「${raw}」已存在`;
    return;
  }
  if (kind === "actions") {
    actionRows.value.push({
      id: null,
      name: raw,
      kind: "reminding",
      isFeeding: false,
      intervalText: "7",
      enabled: true,
      isPreset: false,
      referenced: false,
    });
    addActionName.value = "";
  } else {
    foodRows.value.push({ id: null, name: raw, enabled: true, intervalText: "", isPreset: false, referenced: false });
    addFoodName.value = "";
  }
  error.value = "";
}

async function setActionEnabled(row: ActionRow, enabled: boolean) {
  if (row.id === null) return;
  busy.value = true;
  error.value = "";
  try {
    await setActionEnabledCmd({ id: row.id, enabled });
    row.enabled = enabled;
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function eraseAction(row: ActionRow) {
  if (row.id === null) {
    actionRows.value = actionRows.value.filter((r) => r !== row);
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    await eraseActionCmd({ id: row.id });
    actionRows.value = actionRows.value.filter((r) => r !== row);
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function setFoodEnabled(row: FoodRow, enabled: boolean) {
  if (row.id === null) return;
  busy.value = true;
  error.value = "";
  try {
    await setFoodEnabledCmd({ id: row.id, enabled });
    row.enabled = enabled;
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function eraseFood(row: FoodRow) {
  if (row.id === null) {
    foodRows.value = foodRows.value.filter((r) => r !== row);
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    await eraseFoodCmd({ id: row.id });
    foodRows.value = foodRows.value.filter((r) => r !== row);
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

// ── 批量保存（名字/排序/性质/间隔/喂食标记） ──

async function saveActions() {
  const invalid = validateActionRows(actionRows.value);
  if (invalid) {
    error.value = invalid;
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    for (const input of toActionInputs(actionRows.value)) {
      await saveAction({ input });
    }
    await load();
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function saveFoods() {
  const invalid = validateFoodRows(foodRows.value);
  if (invalid) {
    error.value = invalid;
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    for (const input of toFoodInputs(foodRows.value)) {
      await saveFood({ input });
    }
    await load();
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

// ── 通知 tab 操作（票 06）──

async function saveNotify() {
  const input = toSettings(notifyForm.value, autostart.value);
  if (input === null) {
    notifyError.value = DAYS_AHEAD_ERROR;
    return;
  }
  notifyBusy.value = true;
  notifyError.value = "";
  notifySaved.value = "";
  try {
    const saved = await setSettings({ input });
    notifyForm.value = toForm(saved);
    autostart.value = saved.autostart_enabled;
    notifySaved.value = "已保存";
    emit("changed");
  } catch (e) {
    notifyError.value = String(e);
  } finally {
    notifyBusy.value = false;
  }
}

async function testNotify() {
  notifyError.value = "";
  notifySaved.value = "";
  try {
    const r = await sendTestNotification();
    const parts = [
      r.desktop_ok ? "桌面 ✓" : `桌面 ✗（${r.desktop_error ?? "未知错误"}）`,
      r.pushover === null ? "手机：未配置" : r.pushover.ok ? "手机 ✓" : `手机 ✗（${r.pushover.error}）`,
    ];
    notifySaved.value = `测试结果：${parts.join(" · ")}`;
  } catch (e) {
    notifyError.value = String(e);
  }
}

function onPanelChanged() {
  emit("changed");
}

function onPanelSaved() {
  emit("changed");
}

// ── 数据 tab（票 09）：备份 / 导出 / 数据文件夹 ──
const dataResult = ref("");
const dataError = ref("");
const dataBusy = ref(false);

async function revealFolder() {
  dataError.value = "";
  try {
    await revealDataFolder();
  } catch (e) {
    dataError.value = String(e);
  }
}

async function runBackup() {
  dataBusy.value = true;
  dataError.value = "";
  dataResult.value = "";
  try {
    const path = await backupTo();
    dataResult.value = path ? `已备份到：${path}` : "";
  } catch (e) {
    dataError.value = String(e);
  } finally {
    dataBusy.value = false;
  }
}

async function exportData(format: "csv" | "json") {
  dataBusy.value = true;
  dataError.value = "";
  dataResult.value = "";
  try {
    const path = await exportDataCmd({ format });
    dataResult.value = path ? `已导出到：${path}` : "";
  } catch (e) {
    dataError.value = String(e);
  } finally {
    dataBusy.value = false;
  }
}

// ── 数据 tab：日志区（数据安全二期票 01）──
const recentErrors = ref<string[]>([]);
const abnormalExitText = ref<string | null>(null);

async function loadLogSection() {
  try {
    const [errs, abnormal] = await Promise.all([
      getRecentErrors(),
      getLastAbnormalExit(),
    ]);
    recentErrors.value = errs ?? [];
    abnormalExitText.value = formatAbnormalExit(abnormal);
  } catch {
    // 静默：日志展示失败不影响其他功能区
  }
}

async function openLogs() {
  dataError.value = "";
  try {
    await openLogsFolder();
  } catch (e) {
    dataError.value = String(e);
  }
}

// ── 数据 tab：自动备份区（数据安全二期票 02）──
// 批量保存模式（沿通知页签先例）：开关/目录/保留份数在本地表单上积累，
// 「保存」一次性落库；未生效提示与上次备份状态绑「已保存」的配置值。

const backupConfig = ref<BackupConfigInfo | null>(null);
const autoForm = ref<{ enabled: boolean; backupDir: string | null; keepText: string }>({
  enabled: true,
  backupDir: null,
  keepText: "30",
});
const backupError = ref("");
const backupSaved = ref("");
const backupBusy = ref(false);

function applyBackupConfig(c: BackupConfigInfo | null) {
  if (!c) return;
  backupConfig.value = c;
  autoForm.value = {
    enabled: c.enabled,
    backupDir: c.backup_dir,
    keepText: String(c.keep_count),
  };
}

async function loadBackupSection() {
  try {
    applyBackupConfig(await getBackupConfig());
  } catch {
    // 静默：配置读不出（理论外路径，Rust 侧缺失/损坏都回默认值）不挡其他功能区
  }
}

async function pickBackupDir() {
  backupError.value = "";
  backupSaved.value = "";
  try {
    const dir = await pickBackupDirCmd();
    if (dir) autoForm.value.backupDir = dir;
  } catch (e) {
    backupError.value = String(e);
  }
}

async function saveBackupConfig() {
  const keep = validateKeepCount(autoForm.value.keepText);
  if (keep === null) {
    backupError.value = KEEP_COUNT_ERROR;
    backupSaved.value = "";
    return;
  }
  backupBusy.value = true;
  backupError.value = "";
  backupSaved.value = "";
  try {
    const input: BackupConfigInput = {
      enabled: autoForm.value.enabled,
      backup_dir: autoForm.value.backupDir,
      keep_count: keep,
    };
    // 返回收敛后的生效值（Rust 侧已规整目录/校验份数），回显以它为准
    applyBackupConfig(await setBackupConfig({ input }));
    backupSaved.value = "已保存";
  } catch (e) {
    backupError.value = String(e);
  } finally {
    backupBusy.value = false;
  }
}

// ── 数据 tab：恢复区（数据安全二期票 04，spec D5/D11）──
// 流程：选文件（对话框默认定位备份目录，已设置时）→ restore_preview 校验链
// 产摘要 → 摘要预览（含备份内目录设置值与固定文案）→ 二段确认（与删除记录
// 同待遇）→ restore_apply → 刷新或重启提示。校验与替换全在 Rust 侧，当前库
// 在校验阶段零改动；执行成功后本弹窗数据重拉自新库，首页经 db-restored 事件刷新。

const restoreSummary = ref<RestoreSummary | null>(null);
const restorePath = ref("");
const restoreConfirming = ref(false);
const restoreBusy = ref(false);
const restoreError = ref("");
const restoreResult = ref("");

async function startRestore() {
  restoreError.value = "";
  restoreResult.value = "";
  // 对话框默认定位备份目录（已设置时；读取失败不挡选文件）
  let defaultDir: string | null = null;
  try {
    defaultDir = (await getBackupConfig()).backup_dir;
  } catch {
    // 静默：默认定位是锦上添花
  }
  try {
    const picked = await pickRestoreFile({ defaultDir });
    if (!picked) return; // 用户取消选文件
    restorePath.value = picked;
    restoreBusy.value = true;
    // 校验链 + 摘要（Rust staging 临时库，当前库零改动）；拒绝原因直接展示
    restoreSummary.value = await restorePreview({ path: picked });
    restoreConfirming.value = false; // 每份新摘要都重新走二段确认
  } catch (e) {
    restoreError.value = String(e);
    restoreSummary.value = null;
  } finally {
    restoreBusy.value = false;
  }
}

async function confirmRestoreApply() {
  if (!restoreSummary.value) return;
  // 二段确认（与删除记录同待遇）：第一次进入确认态，第二次才真执行
  if (!restoreConfirming.value) {
    restoreConfirming.value = true;
    return;
  }
  restoreBusy.value = true;
  restoreError.value = "";
  try {
    const outcome = await restoreApply({ path: restorePath.value });
    restoreResult.value = formatRestoreOutcome(outcome);
    restoreSummary.value = null;
    restoreConfirming.value = false;
    // 库已整体替换：弹窗内字典/通知设置重拉自新库；备份配置在库外不受影响
    await load();
    emit("changed");
  } catch (e) {
    restoreError.value = String(e);
  } finally {
    restoreBusy.value = false;
  }
}

function cancelRestore() {
  restoreSummary.value = null;
  restoreConfirming.value = false;
  restoreError.value = "";
}

function eraseTitle(row: { referenced: boolean; isPreset: boolean }): string {
  if (row.isPreset) return "预置项不能删除；可改为停用";
  return row.referenced ? "被历史记录引用，只能停用，不能删除" : "";
}
</script>

<template>
  <div class="overlay" @click.self="$emit('close')">
    <div class="dialog settings-dialog">
      <h3>设置 · 字典管理</h3>

      <div class="tabs">
        <button class="tab tab-actions" :class="{ active: activeTab === 'actions' }" type="button" @click="activeTab = 'actions'">
          操作
        </button>
        <button class="tab tab-foods" :class="{ active: activeTab === 'foods' }" type="button" @click="activeTab = 'foods'">
          食物
        </button>
        <button class="tab tab-locations" :class="{ active: activeTab === 'locations' }" type="button" @click="activeTab = 'locations'">
          地点
        </button>
        <button class="tab tab-notify" :class="{ active: activeTab === 'notify' }" type="button" @click="activeTab = 'notify'">
          通知
        </button>
        <button class="tab tab-data" :class="{ active: activeTab === 'data' }" type="button" @click="activeTab = 'data'">
          数据
        </button>
        <button class="tab tab-update" :class="{ active: activeTab === 'update' }" type="button" @click="activeTab = 'update'">
          更新
        </button>
      </div>

      <!-- 操作 -->
      <div v-if="activeTab === 'actions'" class="tab-body">
        <div v-for="(row, index) in actionRows" :key="row.id ?? `new-${index}`" class="dict-row" :class="{ 'row-disabled': !row.enabled }">
          <span class="movers">
            <button type="button" :disabled="index === 0" @click="moveRow(actionRows, index, -1)">↑</button>
            <button type="button" :disabled="index === actionRows.length - 1" @click="moveRow(actionRows, index, 1)">↓</button>
          </span>
          <input v-model="row.name" class="name-input" type="text" />
          <select v-model="row.kind" class="kind-select" :title="row.kind === 'reminding' ? '提醒类：超期标红并通知' : '仅登记：只记录，永不催促'">
            <option value="reminding">提醒</option>
            <option value="log_only">仅登记</option>
          </select>
          <input
            v-if="row.kind === 'reminding'"
            v-model="row.intervalText"
            class="interval-input"
            type="number"
            min="1"
            title="建议间隔天数：距上次超过它就标红"
          />
          <label class="feeding-flag" title="勾选后记账时弹出食物多选；建议全局只勾一个（不强制）">
            <input v-model="row.isFeeding" type="checkbox" />
            喂食
          </label>
          <span v-if="!row.enabled" class="disabled-chip">已停用</span>
          <button v-if="row.enabled" class="row-btn" type="button" :disabled="row.id === null" @click="setActionEnabled(row, false)">停用</button>
          <button v-else class="row-btn" type="button" @click="setActionEnabled(row, true)">启用</button>
          <button class="row-btn erase-btn" type="button" :disabled="row.referenced || row.isPreset" :title="eraseTitle(row)" @click="eraseAction(row)">
            删除
          </button>
        </div>

        <div class="add-row">
          <input v-model="addActionName" class="add-input" type="text" placeholder="新操作，如：糖水" @keyup.enter="addActionRow('actions')" />
          <button class="add-btn" type="button" @click="addActionRow('actions')">＋ 添加</button>
        </div>

        <p class="hint">「喂食」标记：勾选的操作记账时会弹出食物多选，建议全局只勾一个（不强制）。性质与间隔改完点「保存」，首页卡片红/灰随之变化。</p>
        <div class="dlg-btns">
          <span class="spacer"></span>
          <button class="btn primary" type="button" :disabled="busy" @click="saveActions">保存</button>
        </div>
      </div>

      <!-- 食物 -->
      <div v-else-if="activeTab === 'foods'" class="tab-body">
        <div v-for="(row, index) in foodRows" :key="row.id ?? `new-${index}`" class="dict-row" :class="{ 'row-disabled': !row.enabled }">
          <span class="movers">
            <button type="button" :disabled="index === 0" @click="moveRow(foodRows, index, -1)">↑</button>
            <button type="button" :disabled="index === foodRows.length - 1" @click="moveRow(foodRows, index, 1)">↓</button>
          </span>
          <input v-model="row.name" class="name-input" type="text" />
          <input
            v-model="row.intervalText"
            class="interval-input"
            type="number"
            min="1"
            title="食物建议间隔：距上次喂该食物超过它就单独提醒；留空 = 只按喂食统一周期"
          />
          <span v-if="!row.enabled" class="disabled-chip">已停用</span>
          <button v-if="row.enabled" class="row-btn" type="button" :disabled="row.id === null" @click="setFoodEnabled(row, false)">停用</button>
          <button v-else class="row-btn" type="button" @click="setFoodEnabled(row, true)">启用</button>
          <button class="row-btn erase-btn" type="button" :disabled="row.referenced || row.isPreset" :title="eraseTitle(row)" @click="eraseFood(row)">
            删除
          </button>
        </div>

        <div class="add-row">
          <input v-model="addFoodName" class="add-input" type="text" placeholder="新食物，如：糖水" @keyup.enter="addActionRow('foods')" />
          <button class="add-btn" type="button" @click="addActionRow('foods')">＋ 添加</button>
        </div>

        <p class="hint">设了间隔的食物各自算「距上次」，任一超期喂食块就变红并单独提醒。</p>

        <div class="dlg-btns">
          <span class="spacer"></span>
          <button class="btn primary" type="button" :disabled="busy" @click="saveFoods">保存</button>
        </div>
      </div>

      <!-- 地点 -->
      <div v-else-if="activeTab === 'locations'" class="tab-body">
        <LocationManagerPanel :locations="locations" @saved="onPanelSaved" @changed="onPanelChanged" />
      </div>

      <!-- 通知（票 06）+ 开机自启（票 09）+ Pushover 双通道（反馈第二轮 F4） -->
      <div v-else-if="activeTab === 'notify'" class="tab-body">
        <div class="notify-row">
          <label>
            <input v-model="notifyForm.master" type="checkbox" />
            推送通知（桌面 + 手机）
          </label>
        </div>
        <div class="notify-row">
          <span>手机推送（Pushover）：</span>
          <span v-if="pushoverStatus?.user_found && pushoverStatus?.token_found" class="push-ok">
            已配置（环境变量 PUSHOVER_USER / PUSHOVER_TOKEN）
          </span>
          <span v-else class="push-miss">
            未检测到（需设置环境变量 PUSHOVER_USER / PUSHOVER_TOKEN，配置后重启应用生效）
          </span>
        </div>
        <div class="notify-row">
          <label>
            临近出眠提前
            <input v-model="notifyForm.daysAheadText" class="days-input" type="number" min="0" max="365" />
            天通知
          </label>
        </div>
        <div class="notify-row">
          <label title="Windows 登录后自动启动；随「保存」落库并同步系统启动项">
            <input v-model="autostart" type="checkbox" />
            开机自启
          </label>
        </div>
        <p class="hint">
          超期每天最多提醒一条；冬眠中的窝静音；改预计出眠日后，没发过的提醒按新日期重算。
          总开关关闭时完全静默（不写提醒台账），重开后照常提醒。
        </p>
        <p v-if="notifyError" class="form-error">{{ notifyError }}</p>
        <p v-if="notifySaved" class="saved-hint">{{ notifySaved }}</p>
        <div class="dlg-btns">
          <button class="btn" type="button" :disabled="notifyBusy" @click="testNotify">发送测试通知</button>
          <span class="spacer"></span>
          <button class="btn primary" type="button" :disabled="notifyBusy" @click="saveNotify">保存</button>
        </div>
      </div>

      <!-- 更新（release-update 票 06）：检查更新 / 确认安装 / 升级残留引导 -->
      <div v-else-if="activeTab === 'update'" class="tab-body">
        <UpdatePanel />
      </div>

      <!-- 数据（票 09 手动出口 + 数据安全二期票 02 自动备份区 + 票 01 日志区） -->
      <div v-else-if="activeTab === 'data'" class="tab-body">
        <!-- 自动备份区（数据安全二期票 02）：开关 / 目录 / 保留份数 / 上次备份状态 -->
        <div v-if="backupConfig" class="backup-section">
          <div class="notify-row">
            <label>
              <input v-model="autoForm.enabled" class="auto-enabled-input" type="checkbox" />
              自动备份（每天第一次产生新数据时备份一次，只保留最近 N 份）
            </label>
          </div>
          <div class="notify-row">
            <span>备份目录：</span>
            <span v-if="autoForm.backupDir" class="backup-dir-current">{{ autoForm.backupDir }}</span>
            <span v-else class="backup-dir-current backup-dir-none">未设置</span>
            <button
              class="btn data-btn pick-backup-dir-btn"
              type="button"
              title="系统文件夹选择框选取；可放进网盘同步目录等任意位置"
              @click="pickBackupDir"
            >
              选择目录…
            </button>
          </div>
          <div class="notify-row">
            <label>
              保留份数
              <input
                v-model="autoForm.keepText"
                class="days-input keep-count-input"
                type="number"
                min="1"
                max="365"
                title="备份目录里只留最近 N 份，超出自动删最旧（只清理本应用自动备份的文件）"
              />
              份
            </label>
          </div>
          <p v-if="autoBackupInactive(backupConfig)" class="backup-inactive">
            未设置备份目录，自动备份未生效
          </p>
          <p class="backup-last-status">上次备份：{{ formatLastBackup(backupConfig.last_result) }}</p>
          <p v-if="backupError" class="form-error backup-error">{{ backupError }}</p>
          <p v-if="backupSaved" class="saved-hint backup-saved-hint">{{ backupSaved }}</p>
          <div class="dlg-btns">
            <span class="spacer"></span>
            <button class="btn primary save-backup-btn" type="button" :disabled="backupBusy" @click="saveBackupConfig">
              保存
            </button>
          </div>
        </div>

        <!-- 恢复区（数据安全二期票 04）：位置在自动备份区与手动按钮之间（D11） -->
        <div class="backup-section restore-section">
          <div class="notify-row">
            <button
              class="btn data-btn restore-btn"
              type="button"
              :disabled="restoreBusy"
              title="选一份备份整体替换当前全部数据（恢复前自动快照当前库，可反悔）"
              @click="startRestore"
            >
              从备份恢复…
            </button>
            <span class="restore-hint">整库替换，不是合并——旧数据全部消失，恢复前会自动快照当前库</span>
          </div>
          <p v-if="restoreError" class="form-error restore-error">{{ restoreError }}</p>
          <div v-if="restoreSummary" class="restore-panel">
            <p class="restore-panel-title">备份摘要（请确认没选错文件）</p>
            <div v-for="([label, value], i) in summaryRows(restoreSummary)" :key="i" class="restore-summary-row">
              <span class="restore-summary-label">{{ label }}：</span>
              <span class="restore-summary-value">{{ value }}</span>
            </div>
            <p v-if="summaryLooksSuspicious(restoreSummary)" class="form-error restore-suspicious">
              这份备份里 0 窝 0 条记录——如果与预期不符，可能选错了文件，请勿继续。
            </p>
            <p class="restore-confirm-text">{{ RESTORE_CONFIRM_TEXT }}</p>
            <div class="dlg-btns">
              <button class="btn restore-cancel-btn" type="button" :disabled="restoreBusy" @click="cancelRestore">
                取消
              </button>
              <span class="spacer"></span>
              <button
                class="btn restore-confirm-btn"
                :class="{ confirming: restoreConfirming }"
                type="button"
                :disabled="restoreBusy"
                @click="confirmRestoreApply"
              >
                {{ restoreConfirming ? "再次点击确认恢复" : "确认恢复" }}
              </button>
            </div>
          </div>
          <p v-if="restoreResult" class="data-result restore-result">{{ restoreResult }}</p>
        </div>

        <div class="data-actions">
          <button class="btn data-btn reveal-btn" type="button" title="在资源管理器中打开库文件所在目录" @click="revealFolder">
            打开数据文件夹
          </button>
          <button class="btn data-btn backup-btn" type="button" :disabled="dataBusy" title="把库文件完整拷贝到你选的位置（无需退出）" @click="runBackup">
            安全备份
          </button>
          <button class="btn data-btn export-csv-btn" type="button" :disabled="dataBusy" title="记录流水一行一条，Excel 可直开" @click="exportData('csv')">
            导出 CSV
          </button>
          <button class="btn data-btn export-json-btn" type="button" :disabled="dataBusy" title="全库数据结构化归档" @click="exportData('json')">
            导出 JSON
          </button>
        </div>
        <p v-if="dataResult" class="data-result">{{ dataResult }}</p>
        <p v-if="dataError" class="form-error data-error">{{ dataError }}</p>
        <p class="hint">
          数据都在一个本地 SQLite 库文件里。「安全备份」由应用把库文件完整拷贝到你选的位置，
          应用内一键安全备份无需退出；手动拷贝备份需先从托盘真实退出后再拷。
          导出 CSV / JSON 用于归档带走。「从备份恢复」是应用内整库替换：先看摘要确认，
          恢复前自动快照当前库（可反悔），失败则原库分毫不动。
          自动备份每天第一次产生新数据时备份一次到备份目录，失败过会自动补跑；
          日志按天滚动，自动清理 14 天前的旧日志。
        </p>

        <!-- 日志区（数据安全二期票 01）：打开日志文件夹 / 上次异常退出 / 最近错误摘要 -->
        <div class="log-section">
          <div class="data-actions log-actions">
            <button
              class="btn data-btn open-logs-btn"
              type="button"
              title="在资源管理器中打开日志目录（纯文本按天滚动，自动清理 14 天前的旧日志）"
              @click="openLogs"
            >
              打开日志文件夹
            </button>
          </div>
          <p v-if="abnormalExitText" class="abnormal-exit" title="时间为异常会话的启动时间（异常发生的时刻无从得知）">
            {{ abnormalExitText }}
          </p>
          <div v-if="recentErrors.length" class="recent-errors">
            <p class="recent-errors-title">最近错误（新在上，共 {{ recentErrors.length }} 条）</p>
            <ul class="recent-errors-list">
              <li v-for="(line, i) in recentErrors" :key="i" class="recent-error-line">{{ line }}</li>
            </ul>
          </div>
          <p v-else class="hint recent-errors-empty">最近没有错误记录。</p>
        </div>
      </div>

      <p v-if="error && activeTab !== 'locations'" class="form-error">{{ error }}</p>

      <div class="dlg-btns close-row">
        <span class="spacer"></span>
        <button class="btn" type="button" @click="$emit('close')">关闭</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.settings-dialog {
  width: 640px;
  max-width: 94vw;
  max-height: 86vh;
  overflow-y: auto;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.settings-dialog h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

.tabs {
  display: flex;
  gap: 6px;
  border-bottom: 1px solid var(--border);
  margin-bottom: 14px;
}

.tab {
  border: none;
  background: transparent;
  font: inherit;
  font-size: 14px;
  padding: 6px 14px;
  cursor: pointer;
  color: var(--muted);
  border-bottom: 2px solid transparent;
}

.tab.active {
  color: var(--accent-deep);
  font-weight: 600;
  border-bottom-color: var(--accent);
}

.tab-body {
  min-height: 120px;
}

/* 通知 tab（票 06） */
.notify-row {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-bottom: 10px;
  font-size: 14px;
}

.notify-row label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}

.push-ok {
  color: var(--ok, #2e7d32);
}

.push-miss {
  color: var(--bad);
}

.days-input {
  width: 64px;
  padding: 6px 8px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.saved-hint {
  margin-top: 10px;
  font-size: 13px;
  color: var(--ok, #2e7d32);
}

/* 数据 tab（票 09） */
.data-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}

/* 数据 tab 自动备份区（数据安全二期票 02） */
.backup-section {
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 12px;
  margin-bottom: 14px;
}

.backup-dir-current {
  font-size: 13px;
  word-break: break-all;
}

.backup-dir-none {
  color: var(--muted);
}

.backup-inactive {
  font-size: 13px;
  color: var(--bad);
  margin-bottom: 6px;
}

.backup-last-status {
  font-size: 13px;
  color: var(--muted);
  word-break: break-all;
}

/* 数据 tab 恢复区（数据安全二期票 04） */
.restore-hint {
  font-size: 12px;
  color: var(--muted);
}

.restore-panel {
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  padding: 10px 12px;
  margin-top: 8px;
  background: var(--tile);
}

.restore-panel-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 6px;
}

.restore-summary-row {
  font-size: 13px;
  padding: 1px 0;
}

.restore-summary-label {
  color: var(--muted);
}

.restore-summary-value {
  word-break: break-all;
}

.restore-suspicious {
  margin-top: 8px;
}

.restore-confirm-text {
  margin-top: 8px;
  font-size: 13px;
  color: var(--bad);
}

.restore-confirm-btn.confirming {
  background: var(--bad);
  border-color: var(--bad);
  color: #fff;
  font-weight: 600;
}

.data-btn {
  font-size: 13px;
}

.data-result {
  margin-top: 10px;
  font-size: 13px;
  color: var(--ok, #2e7d32);
  word-break: break-all;
}

/* 数据 tab 日志区（数据安全二期票 01） */
.log-section {
  margin-top: 14px;
  border-top: 1px solid var(--border);
  padding-top: 12px;
}

.log-actions {
  margin-bottom: 8px;
}

.abnormal-exit {
  margin-bottom: 8px;
  font-size: 13px;
  color: var(--bad);
  word-break: break-all;
}

.recent-errors-title {
  font-size: 12px;
  color: var(--muted);
  margin-bottom: 4px;
}

.recent-errors-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 180px;
  overflow-y: auto;
}

.recent-error-line {
  font-size: 12px;
  color: var(--bad);
  word-break: break-all;
  padding: 2px 0;
  border-bottom: 1px dashed var(--border);
}

.dict-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.dict-row.row-disabled {
  opacity: 0.55;
}

.movers {
  display: flex;
  gap: 2px;
}

.movers button {
  border: 1px solid var(--border-strong);
  background: var(--tile);
  border-radius: 6px;
  cursor: pointer;
  font: inherit;
  padding: 4px 8px;
}

.movers button:disabled {
  opacity: 0.35;
  cursor: default;
}

.name-input {
  flex: 1;
  min-width: 90px;
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.kind-select {
  padding: 6px 8px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  font-size: 13px;
  background: var(--card);
  color: var(--text);
}

.interval-input {
  width: 64px;
  padding: 6px 8px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.feeding-flag {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 13px;
  color: var(--muted);
  cursor: pointer;
  white-space: nowrap;
}

.disabled-chip {
  font-size: 11px;
  color: var(--hib);
  background: var(--hib-soft);
  border-radius: 999px;
  padding: 1px 8px;
  white-space: nowrap;
}

.row-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  border-radius: 8px;
  font: inherit;
  font-size: 12px;
  padding: 5px 10px;
  cursor: pointer;
  color: var(--muted);
  white-space: nowrap;
}

.row-btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.row-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.add-row {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}

.add-input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.add-btn {
  border: 1px dashed var(--border-strong);
  background: transparent;
  border-radius: 8px;
  font: inherit;
  padding: 6px 14px;
  cursor: pointer;
  color: var(--muted);
}

.add-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.hint {
  margin-top: 12px;
  font-size: 12px;
  color: var(--muted);
}

.form-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
}

.dlg-btns {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 16px;
}

.close-row {
  border-top: 1px solid var(--border);
  padding-top: 12px;
}

.spacer {
  flex: 1;
}

.btn {
  padding: 7px 18px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
  font-weight: 600;
}

.btn.primary:hover {
  background: var(--accent-deep);
}

.btn:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
