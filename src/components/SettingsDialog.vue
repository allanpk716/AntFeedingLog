<script setup lang="ts">
/**
 * 设置弹窗（票 04 字典管理 + 票 06 通知）：操作 / 食物 / 地点 / 通知 四个 tab。
 * - 操作行可编辑：名字、性质（提醒/仅登记）、建议间隔（提醒类显示）、
 *   「喂食」标记（带提示，允许编辑不强制唯一）、停用/启用、删除
 *   （被历史记录/提醒台账引用时删除禁用，只能停用——规则 10）。
 * - 食物行：名字、停用/启用、删除（被引用禁用）。
 * - 地点 tab 复用 LocationManagerPanel。
 * - 通知 tab（票 06）：总开关/超期/冬眠三开关 + 临近出眠提前天数 +
 *   「发送测试通知」按钮（排障用）；保存整体落库，autostart 本票透传不动。
 *
 * 行级 停用/启用/删除 即时落库并抛 changed（外层刷新首页，卡片红/灰随之变化）；
 * 名字/排序/性质/间隔/喂食标记在本地行上积累，「保存」一次性按行序落库（sort=行下标），
 * 成功后重拉字典并抛 changed。停用项整行置灰。
 */
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, CareActionItem, FoodItem, LocationItem } from "../types";
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
import LocationManagerPanel from "./LocationManagerPanel.vue";

type Tab = "actions" | "foods" | "locations" | "notify";

const emit = defineEmits<{ close: []; changed: [] }>();

const activeTab = ref<Tab>("actions");
const actionRows = ref<ActionRow[]>([]);
const foodRows = ref<FoodRow[]>([]);
const locations = ref<LocationItem[]>([]);
const addActionName = ref("");
const addFoodName = ref("");
const error = ref("");
const busy = ref(false);

// ── 通知 tab（票 06）──
const notifyForm = ref<NotifySettingsForm>({ master: true, overdue: true, hibernation: true, daysAheadText: "7" });
const autostart = ref(true);
const notifyError = ref("");
const notifySaved = ref("");
const notifyBusy = ref(false);

async function load() {
  const [actions, foods, locs, s] = await Promise.all([
    invoke<CareActionItem[]>("list_actions"),
    invoke<FoodItem[]>("list_foods"),
    invoke<LocationItem[]>("list_locations"),
    invoke<AppSettings>("get_settings"),
  ]);
  actionRows.value = buildActionRows(actions);
  foodRows.value = buildFoodRows(foods);
  locations.value = locs;
  notifyForm.value = toForm(s);
  autostart.value = s.autostart_enabled;
}

onMounted(async () => {
  try {
    await load();
  } catch (e) {
    error.value = String(e);
  }
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
      referenced: false,
    });
    addActionName.value = "";
  } else {
    foodRows.value.push({ id: null, name: raw, enabled: true, referenced: false });
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
    await invoke("send_test_notification");
    notifySaved.value = "测试通知已发出，看一下系统通知";
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

function eraseTitle(referenced: boolean): string {
  return referenced ? "被历史记录引用，只能停用，不能删除" : "";
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
          <button class="row-btn erase-btn" type="button" :disabled="row.referenced" :title="eraseTitle(row.referenced)" @click="eraseAction(row)">
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
          <span v-if="!row.enabled" class="disabled-chip">已停用</span>
          <button v-if="row.enabled" class="row-btn" type="button" :disabled="row.id === null" @click="setFoodEnabled(row, false)">停用</button>
          <button v-else class="row-btn" type="button" @click="setFoodEnabled(row, true)">启用</button>
          <button class="row-btn erase-btn" type="button" :disabled="row.referenced" :title="eraseTitle(row.referenced)" @click="eraseFood(row)">
            删除
          </button>
        </div>

        <div class="add-row">
          <input v-model="addFoodName" class="add-input" type="text" placeholder="新食物，如：糖水" @keyup.enter="addActionRow('foods')" />
          <button class="add-btn" type="button" @click="addActionRow('foods')">＋ 添加</button>
        </div>

        <div class="dlg-btns">
          <span class="spacer"></span>
          <button class="btn primary" type="button" :disabled="busy" @click="saveFoods">保存</button>
        </div>
      </div>

      <!-- 地点 -->
      <div v-else-if="activeTab === 'locations'" class="tab-body">
        <LocationManagerPanel :locations="locations" :show-cancel="false" @saved="onPanelSaved" @changed="onPanelChanged" />
      </div>

      <!-- 通知（票 06） -->
      <div v-else class="tab-body">
        <div class="notify-row">
          <label class="switch-label">
            <input v-model="notifyForm.master" type="checkbox" />
            系统通知总开关
          </label>
        </div>
        <div class="notify-row notify-sub" :class="{ 'notify-sub-off': !notifyForm.master }">
          <label>
            <input v-model="notifyForm.overdue" type="checkbox" :disabled="!notifyForm.master" />
            超期提醒（喂食 / 垃圾清理等提醒类）
          </label>
          <label>
            <input v-model="notifyForm.hibernation" type="checkbox" :disabled="!notifyForm.master" />
            冬眠提醒（临近出眠 / 出眠日）
          </label>
        </div>
        <div class="notify-row">
          <label>
            临近出眠提前
            <input v-model="notifyForm.daysAheadText" class="days-input" type="number" min="0" max="365" />
            天通知
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

.notify-sub {
  padding-left: 24px;
}

.notify-sub-off {
  opacity: 0.55;
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
