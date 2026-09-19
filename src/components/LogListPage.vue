<script setup lang="ts">
/**
 * 记录列表页（票 08）：筛选行（窝 / 启用中操作 / 时间范围 / 备注关键词，组合生效）
 * + 流水表格（时间、窝、操作、食物、备注）+ 分页（加载更多）+ 行内编辑弹窗
 * （时间 / 操作 / 食物多选 / 备注）+ 两段确认删除。
 * 停用操作/食物按规则 10：历史照常显示；编辑表单原引用可保留（标「已停用」），
 * 新挂停用项前后端双重拒绝。任何编辑/删除成功抛 changed → 外层 refresh，
 * 首页「距上次」与超期态即时重算（数据驱动）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { deleteLog, listActions, listColonies, listFoods, listLogs, updateLog } from "../lib/ipc";
import { watchDataVersion } from "../lib/versionSync";
import type { CareActionItem, Colony, FoodItem, LogPage, LogRow } from "../types";
import {
  buildLogFilter,
  buildUpdateInput,
  canPickAction,
  canPickFood,
  dictLabel,
  emptyFilterForm,
  filterFormError,
  formatLogTime,
  nextOffset,
  type LogFilterForm,
} from "../lib/loglist";

const emit = defineEmits<{ changed: [] }>();

const colonies = ref<Colony[]>([]);
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

async function load(offset: number) {
  loading.value = true;
  try {
    const res = await listLogs({
      filter: buildLogFilter(form.value, offset),
    });
    page.value =
      offset === 0
        ? res
        : { total: res.total, rows: [...(page.value?.rows ?? []), ...res.rows] };
    pageError.value = "";
  } catch (e) {
    pageError.value = String(e);
  } finally {
    loading.value = false;
  }
}

function onColonyChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.colonyId = v === "" ? null : Number(v);
}

function onActionChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.actionId = v === "" ? null : Number(v);
}

function applyFilters() {
  const err = filterFormError(form.value);
  filterError.value = err;
  if (err !== "") return;
  confirmDeleteId.value = null;
  void load(0);
}

function resetFilters() {
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
    await updateLog({
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
    await deleteLog({ id: row.id });
    emit("changed");
    await load(0);
  } catch (e) {
    pageError.value = String(e);
  }
}

/** 全量重拉：字典三件套 + 首页流水（筛选表单原样保留）。挂载与版本广播
 * （webui-checkin 票 06，别端记录后本页开着就刷新）共用这一个入口。 */
async function reloadAll() {
  try {
    const [cols, acts, fds] = await Promise.all([
      listColonies(),
      listActions(),
      listFoods(),
    ]);
    colonies.value = cols;
    actions.value = acts;
    foods.value = fds;
  } catch (e) {
    pageError.value = String(e);
  }
  await load(0);
}

/** 版本广播退订柄（票 06；页签卸载时调用）。 */
let unwatchVersion: (() => void) | null = null;

onMounted(() => {
  void reloadAll();
  unwatchVersion = watchDataVersion(() => void reloadAll());
});

onBeforeUnmount(() => {
  unwatchVersion?.();
});
</script>

<template>
  <div class="log-list">
    <div class="container">
      <div class="filters">
        <label class="f-label">窝</label>
        <select class="f-colony" :value="form.colonyId ?? ''" @change="onColonyChange">
          <option value="">全部</option>
          <option v-for="c in colonies" :key="c.id" :value="c.id">{{ c.name }}</option>
        </select>

        <label class="f-label">操作</label>
        <select class="f-action" :value="form.actionId ?? ''" @change="onActionChange">
          <option value="">全部</option>
          <option v-for="a in enabledActions" :key="a.id" :value="a.id">{{ a.name }}</option>
        </select>

        <label class="f-label">从</label>
        <input class="f-start" v-model="form.start" type="date" />
        <label class="f-label">到</label>
        <input class="f-end" v-model="form.end" type="date" />

        <input
          class="f-keyword"
          v-model="form.keyword"
          type="search"
          placeholder="搜备注关键词"
          @keyup.enter="applyFilters"
        />
        <button class="apply-btn" type="button" :disabled="loading" @click="applyFilters">
          查询
        </button>
        <button class="reset-btn" type="button" @click="resetFilters">重置</button>
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
              <td class="c-colony">{{ r.colony_name }}</td>
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
          <input v-model="editTime" class="time-input" type="datetime-local" />

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
  padding: 0 20px 80px;
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

.apply-btn,
.reset-btn {
  padding: 6px 16px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.apply-btn {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
  font-weight: 600;
}

.apply-btn:hover {
  background: var(--accent-deep);
}

.apply-btn:disabled {
  opacity: 0.6;
  cursor: default;
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
.dialog input[type="datetime-local"],
.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
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
