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
 * - 更新 tab（release-update 票 06）：当前版本 / 立即检查更新 / 确认下载安装，
 *   全部走 Tauri command；升级未完成残留的引导也挂在本节顶（UpdatePanel）。
 *
 * 行级 停用/启用/删除 即时落库并抛 changed（外层刷新首页，卡片红/灰随之变化）；
 * 名字/排序/性质/间隔/喂食标记在本地行上积累，「保存」一次性按行序落库（sort=行下标），
 * 成功后重拉字典并抛 changed。停用项整行置灰。
 */
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, CareActionItem, FoodItem, LocationItem, PushoverStatus, TestNotifyOutcome, AbnormalExitInfo } from "../types";
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
    invoke<CareActionItem[]>("list_actions"),
    invoke<FoodItem[]>("list_foods"),
    invoke<LocationItem[]>("list_locations"),
    invoke<AppSettings>("get_settings"),
    invoke<PushoverStatus>("pushover_status"),
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
    await invoke("set_action_enabled", { id: row.id, enabled });
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
    await invoke("erase_action", { id: row.id });
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
    await invoke("set_food_enabled", { id: row.id, enabled });
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
    await invoke("erase_food", { id: row.id });
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
      await invoke("save_action", { input });
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
      await invoke("save_food", { input });
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
    const saved = await invoke<AppSettings>("set_settings", { input });
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
    const r = await invoke<TestNotifyOutcome>("send_test_notification");
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
    await invoke<string>("reveal_data_folder");
  } catch (e) {
    dataError.value = String(e);
  }
}

async function runBackup() {
  dataBusy.value = true;
  dataError.value = "";
  dataResult.value = "";
  try {
    const path = await invoke<string | null>("backup_to");
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
    const path = await invoke<string | null>("export_data", { format });
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
      invoke<string[]>("get_recent_errors"),
      invoke<AbnormalExitInfo | null>("get_last_abnormal_exit"),
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
    await invoke<string>("open_logs_folder");
  } catch (e) {
    dataError.value = String(e);
  }
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

      <!-- 数据（票 09）：打开数据文件夹 / 安全备份 / 导出归档 -->
      <div v-else-if="activeTab === 'data'" class="tab-body">
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
          导出 CSV / JSON 用于归档带走，不是恢复通道（恢复 = 把备份的 .db 拷回数据文件夹）。
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
