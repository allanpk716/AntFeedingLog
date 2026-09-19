<script setup lang="ts">
/**
 * 记录列表页（票 08）：筛选行（地点 / 窝级联 / 启用中操作 / 时间范围 / 备注关键词，
 * 组合生效；下拉/日期变更即查 + 关键词 300ms 防抖，无「查询」按钮）
 * + 流水表格（时间、窝（下挂地点小字）、操作、食物、备注）+ 分页（加载更多）
 * + 行内编辑弹窗（标记日历 DateTimeField / 操作 / 食物多选 / 备注 + 当天重复黄条，
 * 月数据传 excludeLogId 排除自身）+ 两段确认删除。
 * 停用操作/食物按规则 10：历史照常显示；编辑表单原引用可保留（标「已停用」），
 * 新挂停用项前后端双重拒绝。任何编辑/删除成功抛 changed → 外层 refresh，
 * 首页「距上次」与超期态即时重算（数据驱动）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type {
  CareActionItem,
  Colony,
  FoodItem,
  LocationItem,
  LogPage,
  LogRow,
  MonthDayRecords,
} from "../types";
import {
  buildLogFilter,
  buildUpdateInput,
  canPickAction,
  canPickFood,
  colonyOptionsFor,
  dictLabel,
  emptyFilterForm,
  filterFormError,
  formatLogTime,
  nextOffset,
  type LogFilterForm,
} from "../lib/loglist";
import { todayIso } from "../lib/dates";
import { buildMarkers, duplicateInfo, dupWarningText } from "../lib/monthview";
import DatePickerPop from "./DatePickerPop.vue";
import DateTimeField from "./DateTimeField.vue";

const emit = defineEmits<{ changed: [] }>();

const colonies = ref<Colony[]>([]);
const locations = ref<LocationItem[]>([]);
const actions = ref<CareActionItem[]>([]);
const foods = ref<FoodItem[]>([]);

const form = ref<LogFilterForm>(emptyFilterForm());
const filterError = ref("");
const page = ref<LogPage | null>(null);
const pageError = ref("");
const loading = ref(false);

const rows = computed(() => page.value?.rows ?? []);
const moreOffset = computed(() =>
  page.value === null ? null : nextOffset(rows.value.length, page.value.total),
);
/** 筛选下拉只列启用中的操作（规则 10：停用项不进筛选入口） */
const enabledActions = computed(() => actions.value.filter((a) => a.enabled));

/** 响应序号守卫（复审修正）：即改即查后连续请求变多，旧查询后到不得覆盖新结果 */
let loadSeq = 0;

async function load(offset: number) {
  const seq = ++loadSeq;
  loading.value = true;
  try {
    const res = await invoke<LogPage>("list_logs", {
      filter: buildLogFilter(form.value, offset),
    });
    if (seq !== loadSeq) return; // 过期响应，丢弃
    page.value =
      offset === 0
        ? res
        : { total: res.total, rows: [...(page.value?.rows ?? []), ...res.rows] };
    pageError.value = "";
  } catch (e) {
    if (seq !== loadSeq) return;
    pageError.value = String(e);
  } finally {
    if (seq === loadSeq) loading.value = false; // 被更新请求接管的收尾不动 loading
  }
}

/** 级联窝选项（交互第三轮 #1）：选了地点，窝下拉只列该地点的窝 */
const colonyOptions = computed(() => colonyOptionsFor(colonies.value, form.value.locationId));

function onLocationChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.locationId = v === "" ? null : Number(v);
  // 原选中窝不在新地点 → 清空为「全部」
  if (
    form.value.colonyId !== null &&
    !colonyOptions.value.some((c) => c.id === form.value.colonyId)
  ) {
    form.value.colonyId = null;
  }
  applyFilters(); // 即改即查（#4）
}

function onColonyChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.colonyId = v === "" ? null : Number(v);
  applyFilters();
}

function onActionChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.actionId = v === "" ? null : Number(v);
  applyFilters();
}

/** 日期变更即查（DatePickerPop 选中即 emit；命名中转，模板内联箭头在 vue-tsc 下推断不稳） */
function onStartDatePick(iso: string) {
  form.value.start = iso;
  applyFilters();
}

function onEndDatePick(iso: string) {
  form.value.end = iso;
  applyFilters();
}

/** 关键词防抖 300ms（#4，复审修正）：输入即时同步 form.keyword、只防抖查询动作——
 * 若同步也挂到回调里，重置后 form.keyword 仍是空串，Vue 不 patch 输入框，旧词残留界面 */
let kwTimer: ReturnType<typeof setTimeout> | undefined;
function onKeywordInput(e: Event) {
  // IME 组词期（拼音组词中）不同步不防抖（v-model/vModelText 同款守卫）：
  // isComposing 在 InputEvent 上（评审原稿 cast 到 HTMLInputElement 类型错且运行时恒 undefined）；
  // happy-dom 的 input 事件 isComposing 为 undefined，=== true 判定不影响测试
  if ((e as InputEvent).isComposing === true) return;
  form.value.keyword = (e.target as HTMLInputElement).value;
  clearTimeout(kwTimer);
  kwTimer = setTimeout(() => {
    kwTimer = undefined;
    applyFilters();
  }, 300);
}

onBeforeUnmount(() => clearTimeout(kwTimer));

function applyFilters() {
  const err = filterFormError(form.value);
  filterError.value = err;
  if (err !== "") return;
  confirmDeleteId.value = null;
  void load(0);
}

function resetFilters() {
  clearTimeout(kwTimer); // 复审 #8/#9：防抖挂起时点重置，先撤挂起回调，防 300ms 后旧词回写再查
  kwTimer = undefined;
  form.value = emptyFilterForm();
  filterError.value = "";
  confirmDeleteId.value = null;
  void load(0);
}

// ── 行内编辑弹窗 ─────────────────────────────────────────────────────────

const editing = ref<LogRow | null>(null);
const editTime = ref("");
const editActionId = ref<number | null>(null);
const editFoodIds = ref<number[]>([]);
const editNote = ref("");
const editError = ref("");
const editBusy = ref(false);

const editIsFeeding = computed(
  () => actions.value.find((a) => a.id === editActionId.value)?.is_feeding ?? false,
);

// ── 编辑弹窗：标记日历 + 当天重复黄条（#8 编辑场景 Q10-C）─────────────────

const editMonthRows = ref<MonthDayRecords[]>([]);
const editViewMonth = ref({ year: 2026, month: 9 });
let editMonthSeq = 0; // 复审 #10：快速翻月旧响应后到会污染标记/黄条，序号守卫丢弃过期响应

/** 日历标记：当前操作橙点、其它操作灰点——名字表取 actions 全量（复审 #1/#2 同口径，
 * 缺名操作被跳过会让灰点与 tip 整体失效） */
const editMarkers = computed(() => {
  const names = new Map(actions.value.map((a) => [a.id, a.name]));
  return buildMarkers(editMonthRows.value, editActionId.value ?? -1, names);
});

/** 选中日期重复判定（时间字段前 10 位 = 日期；viewMonth 由跨月 watch 与时间值保持同步） */
const editDup = computed(() =>
  editing.value === null || editActionId.value === null
    ? null
    : duplicateInfo(
        editMonthRows.value,
        editViewMonth.value.year,
        editViewMonth.value.month,
        editTime.value.slice(0, 10),
        editActionId.value,
      ),
);

const editDupText = computed(() => {
  if (editing.value === null) return "";
  // 复审 #17：查真实操作名（切换操作后黄条跟随），查不到兜底「该操作」
  const actionName = actions.value.find((a) => a.id === editActionId.value)?.name ?? "该操作";
  return dupWarningText(editDup.value, editTime.value.slice(0, 10), todayIso(), actionName);
});

/** 复审修正：编辑场景传 excludeLogId=当前记录 id——否则正在编辑的这条被算成重复，
 * 只改备注也误报黄条 */
async function loadEditMonth(y: number, m: number) {
  editViewMonth.value = { year: y, month: m };
  const row = editing.value;
  if (row === null) return;
  const seq = ++editMonthSeq;
  try {
    const res = await invoke<MonthDayRecords[]>("colony_month_records", {
      colonyId: row.colony_id,
      year: y,
      month: m,
      excludeLogId: row.id,
    });
    if (seq !== editMonthSeq) return; // 过期响应，丢弃
    editMonthRows.value = res;
  } catch {
    if (seq === editMonthSeq) editMonthRows.value = []; // 标记是增强，失败静默（提交校验权威在后端）
  }
}

/** DateTimeField 的 month 事件 → 拉该月数据（显式类型，模板内联箭头在 vue-tsc 下推断不稳） */
function onEditMonth(view: { year: number; month: number }) {
  void loadEditMonth(view.year, view.month);
}

// 复审 #15：编辑时间跨月（现在/±10分/选日期）时月份重同步（黄条判定依赖 viewMonth）
watch(
  () => editTime.value.slice(0, 7),
  (ym, old) => {
    if (ym !== old) void loadEditMonth(+ym.slice(0, 4), +ym.slice(5, 7));
  },
);

/** 库内 "2026-09-17 21:00:00" → datetime-local 值 "2026-09-17T21:00"；脏值原样兜底 */
function toLocalInput(occurredAt: string): string {
  const s = occurredAt.trim().replace("T", " ");
  return s.length >= 16 ? s.slice(0, 16).replace(" ", "T") : s;
}

function openEdit(row: LogRow) {
  editing.value = row;
  editTime.value = toLocalInput(row.occurred_at);
  editActionId.value = row.action_id;
  editFoodIds.value = [...row.food_ids];
  editNote.value = row.note;
  editError.value = "";
  confirmDeleteId.value = null;
  editMonthRows.value = []; // 清上一次弹窗的月数据，防别的窝/月份旧行瞬间误染黄条
  // 拉编辑所在月的标记数据（watch 只在跨月变化时触发，首次打开要显式拉一次）
  void loadEditMonth(+editTime.value.slice(0, 4), +editTime.value.slice(5, 7));
}

function closeEdit() {
  editing.value = null;
}

function onEditActionChange(e: Event) {
  editActionId.value = Number((e.target as HTMLSelectElement).value);
}

function toggleFood(id: number) {
  editFoodIds.value = editFoodIds.value.includes(id)
    ? editFoodIds.value.filter((x) => x !== id)
    : [...editFoodIds.value, id];
}

/** 食物 chip 是否可选：停用项仅原引用可保留（新挂停用拒绝，与 Rust 同口径） */
function pickable(f: FoodItem): boolean {
  return editing.value !== null && canPickFood(f, editing.value.food_ids);
}

async function saveEdit() {
  if (editing.value === null || editActionId.value === null) return;
  editBusy.value = true;
  editError.value = "";
  try {
    await invoke("update_log", {
      id: editing.value.id,
      input: buildUpdateInput({
        happenedAt: editTime.value,
        actionId: editActionId.value,
        foodIds: editIsFeeding.value ? editFoodIds.value : [],
        note: editNote.value,
      }),
    });
    editing.value = null;
    emit("changed");
    await load(0);
  } catch (e) {
    editError.value = String(e);
  } finally {
    editBusy.value = false;
  }
}

// ── 删除（两段确认：第一次进入确认态，第二次才真删）────────────────────

const confirmDeleteId = ref<number | null>(null);

async function requestDelete(row: LogRow) {
  if (confirmDeleteId.value !== row.id) {
    confirmDeleteId.value = row.id;
    return;
  }
  confirmDeleteId.value = null;
  try {
    await invoke("delete_log", { id: row.id });
    emit("changed");
    await load(0);
  } catch (e) {
    pageError.value = String(e);
  }
}

onMounted(async () => {
  try {
    const [cols, acts, fds, locs] = await Promise.all([
      invoke<Colony[]>("list_colonies"),
      invoke<CareActionItem[]>("list_actions"),
      invoke<FoodItem[]>("list_foods"),
      invoke<LocationItem[]>("list_locations"),
    ]);
    colonies.value = cols;
    actions.value = acts;
    foods.value = fds;
    locations.value = locs;
  } catch (e) {
    pageError.value = String(e);
  }
  await load(0);
});
</script>

<template>
  <div class="log-list">
    <div class="container">
      <div class="filters">
        <label class="f-label">地点</label>
        <select class="f-location" :value="form.locationId ?? ''" @change="onLocationChange">
          <option value="">全部</option>
          <option v-for="l in locations" :key="l.id" :value="l.id">{{ l.name }}</option>
        </select>

        <label class="f-label">窝</label>
        <select class="f-colony" :value="form.colonyId ?? ''" @change="onColonyChange">
          <option value="">全部</option>
          <option v-for="c in colonyOptions" :key="c.id" :value="c.id">{{ c.name }}</option>
        </select>

        <label class="f-label">操作</label>
        <select class="f-action" :value="form.actionId ?? ''" @change="onActionChange">
          <option value="">全部</option>
          <option v-for="a in enabledActions" :key="a.id" :value="a.id">{{ a.name }}</option>
        </select>

        <label class="f-label">从</label>
        <DatePickerPop
          :model-value="form.start"
          placeholder="开始日期"
          @update:model-value="onStartDatePick"
        />
        <label class="f-label">到</label>
        <DatePickerPop
          :model-value="form.end"
          placeholder="结束日期"
          @update:model-value="onEndDatePick"
        />

        <input
          class="f-keyword"
          :value="form.keyword"
          type="search"
          placeholder="搜备注 · 输入即查"
          @input="onKeywordInput"
        />
        <button class="reset-btn" type="button" @click="resetFilters">重置</button>
        <span class="auto-note">⚡ 条件变更即查询</span>
      </div>
      <p v-if="filterError" class="filter-error">{{ filterError }}</p>

      <p v-if="pageError" class="page-error">{{ pageError }}</p>

      <p class="total-note">共 {{ page?.total ?? 0 }} 条记录</p>

      <div class="table-wrap">
        <table class="log-table">
          <thead>
            <tr>
              <th>时间</th>
              <th>窝</th>
              <th>操作</th>
              <th>食物</th>
              <th>备注</th>
              <th class="th-ops"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in rows" :key="r.id" class="log-row" :data-log-id="r.id">
              <td class="c-time">{{ formatLogTime(r.occurred_at) }}</td>
              <td class="c-colony">
                {{ r.colony_name }}<span class="loc">{{ r.location_name ?? "未分组" }}</span>
              </td>
              <td class="c-action">{{ r.action_name }}</td>
              <td class="c-foods">{{ r.food_names.length > 0 ? r.food_names.join("、") : "—" }}</td>
              <td class="c-note">{{ r.note !== "" ? r.note : "—" }}</td>
              <td class="c-ops">
                <button class="row-btn edit-btn" type="button" @click="openEdit(r)">编辑</button>
                <button
                  class="row-btn delete-btn"
                  :class="{ confirming: confirmDeleteId === r.id }"
                  type="button"
                  @click="requestDelete(r)"
                >
                  {{ confirmDeleteId === r.id ? "确认删除？" : "删除" }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
        <p v-if="rows.length === 0 && !loading" class="empty">没有符合条件的记录</p>
      </div>

      <button
        v-if="moreOffset !== null"
        class="more-btn"
        type="button"
        :disabled="loading"
        @click="void load(moreOffset)"
      >
        加载更多（已载 {{ rows.length }} / {{ page?.total ?? 0 }}）
      </button>

      <div v-if="editing !== null" class="overlay" @click.self="closeEdit">
        <div class="dialog edit-dialog">
          <h3>编辑记录 · {{ editing.colony_name }}</h3>

          <div class="field-label">发生时间（可补录）</div>
          <DateTimeField v-model="editTime" :markers="editMarkers" @month="onEditMonth" />
          <p v-if="editDupText !== ''" class="dup-warn">⚠ {{ editDupText }}</p>

          <div class="field-label">操作</div>
          <select class="action-select" :value="editActionId ?? ''" @change="onEditActionChange">
            <option v-for="a in actions" :key="a.id" :value="a.id" :disabled="!canPickAction(a, editing.action_id)">
              {{ dictLabel(a.name, a.enabled) }}
            </option>
          </select>

          <template v-if="editIsFeeding">
            <div class="field-label">食物（可多选；停用项仅原值可保留）</div>
            <div class="foods">
              <button
                v-for="f in foods"
                :key="f.id"
                class="food"
                :class="{ selected: editFoodIds.includes(f.id), off: !f.enabled }"
                type="button"
                :disabled="!pickable(f)"
                :title="!f.enabled ? (pickable(f) ? '已停用 · 保留原值' : '已停用 · 不能新选') : ''"
                @click="toggleFood(f.id)"
              >
                {{ dictLabel(f.name, f.enabled) }}
              </button>
            </div>
          </template>

          <div class="field-label">备注（可选）</div>
          <textarea v-model="editNote" class="note-input" placeholder="备注"></textarea>

          <p v-if="editError" class="form-error">{{ editError }}</p>

          <div class="dlg-btns">
            <button class="btn cancel-btn" type="button" @click="closeEdit">取消</button>
            <button class="btn primary save-btn" type="button" :disabled="editBusy" @click="saveEdit">
              保存
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.log-list {
  --card: #ffffff;
  --tile: #faf8f5;
  --text: #2c2822;
  --muted: #8f887d;
  --border: #e8e2d8;
  --border-strong: #d8d1c4;
  --accent: #d97706;
  --accent-deep: #b45309;
  --accent-soft: #fdf1de;
  --bad: #d13d3d;
  --bad-soft: #fcebeb;
  --shadow: 0 1px 2px rgba(60, 50, 30, 0.05), 0 4px 14px rgba(60, 50, 30, 0.06);
  --overlay: rgba(40, 35, 25, 0.35);
}

.container {
  max-width: 1080px;
  margin: 0 auto;
  padding: 0 20px 32px;
}

.filters {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 20px 0 4px;
  flex-wrap: wrap;
}

.f-label {
  font-size: 13px;
  color: var(--muted);
}

.filters select,
.filters input {
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 9px;
  font: inherit;
  font-size: 13px;
  background: var(--card);
  color: var(--text);
}

.f-keyword {
  min-width: 160px;
}

.reset-btn {
  padding: 6px 16px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.reset-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

/* 与既有下拉/输入对齐（DatePickerPop 自带 8px 圆角、独立配色变量兜底） */
.filters .dp-trigger {
  border-radius: 9px;
}

.auto-note {
  font-size: 11px;
  color: var(--accent-deep);
  background: var(--accent-soft);
  padding: 1px 10px;
  border-radius: 999px;
  white-space: nowrap;
}

.filter-error {
  margin-top: 10px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--bad-soft);
  color: var(--bad);
  font-size: 13px;
}

.page-error {
  margin-top: 12px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--bad-soft);
  color: var(--bad);
  font-size: 13px;
}

.total-note {
  margin-top: 14px;
  font-size: 13px;
  color: var(--muted);
}

.table-wrap {
  margin-top: 8px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow: var(--shadow);
  overflow-x: auto;
}

.log-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

.log-table th {
  text-align: left;
  font-size: 12px;
  color: var(--muted);
  font-weight: 600;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  white-space: nowrap;
}

.log-table td {
  padding: 9px 14px;
  border-bottom: 1px solid var(--border);
  vertical-align: top;
}

.log-table tr:last-child td {
  border-bottom: none;
}

.c-time {
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.c-colony {
  white-space: nowrap;
  font-weight: 600;
}

/* 窝名下挂地点小字（交互第三轮 #5）；未分组显示占位 */
.c-colony .loc {
  display: block;
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
}

.c-foods,
.c-note {
  color: var(--text);
}

.c-note {
  color: var(--muted);
}

.th-ops {
  width: 1%;
}

.c-ops {
  white-space: nowrap;
  text-align: right;
}

.row-btn {
  padding: 4px 10px;
  margin-left: 6px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--card);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
}

.row-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.delete-btn.confirming {
  background: var(--bad-soft);
  border-color: var(--bad);
  color: var(--bad);
  font-weight: 600;
}

.empty {
  padding: 22px;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.more-btn {
  margin-top: 14px;
  width: 100%;
  padding: 10px;
  border: 1.5px dashed var(--border-strong);
  border-radius: 12px;
  background: transparent;
  cursor: pointer;
  color: var(--muted);
  font: inherit;
  font-size: 13px;
}

.more-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

/* 编辑弹窗（视觉基线同 FeedDialog） */
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.dialog {
  width: 400px;
  max-width: 92vw;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.dialog h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

.field-label {
  font-size: 12px;
  color: var(--muted);
  margin: 12px 0 6px;
}

.foods {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.food {
  padding: 6px 14px;
  border: 1px solid var(--border-strong);
  border-radius: 999px;
  background: var(--tile);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
  color: var(--text);
}

.food.selected {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent-deep);
  font-weight: 600;
}

.food.off:not(:disabled) {
  border-style: dashed;
}

.food:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.dialog select,
.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

/* DateTimeField 日历触发器与原 datetime-local 输入同占满行宽 */
.edit-dialog :deep(.dp-trigger) {
  width: 100%;
}

/* 黄条：选中日已有同操作记录提醒（不拦提交，视觉基线 mocks/mock-c-calendar.html） */
.dup-warn {
  margin-top: 6px;
  padding: 7px 10px;
  border-radius: 9px;
  background: #fdf1de;
  border: 1px solid #f3d9a8;
  color: #b45309;
  font-size: 12px;
}

.dialog select option:disabled {
  color: var(--muted);
}

.dialog textarea {
  height: 56px;
  resize: none;
  margin-top: 2px;
}

.form-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
}

.dlg-btns {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
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
