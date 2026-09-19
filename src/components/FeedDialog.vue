<script lang="ts">
/**
 * 票 04：撤食反馈判定的纯函数（导出供测试；涉及路径受限，不进 lib/）。
 * 判定口径 spec F6 / 决策 D17：所选含易腐且全部有有效间隔时，按「发生时刻 + 最短间隔」
 * 分新鲜/已逾期两档文案；脏数据（易腐无有效间隔）与非易腐一律不给将来时刻的承诺。
 */
/** 结构化探针：FoodItem 满足它（测试 fixture 可只给这两个字段） */
export interface PerishableProbe {
  perishable?: boolean;
  retrieval_hours?: number | null;
}

export interface RetrievalFeedback {
  kind: "none" | "fresh" | "overdue";
  /** kind=fresh：最短撤食间隔（小时）；其余 0 */
  hours: number;
}

/** 撤食间隔合法域：1–168 整数（与设置必填守护/后端同口径） */
const RETRIEVAL_HOURS_MIN = 1;
const RETRIEVAL_HOURS_MAX = 168;
const MS_PER_HOUR = 3_600_000;

function validRetrievalHours(v: number | null | undefined): v is number {
  return typeof v === "number" && Number.isInteger(v) && v >= RETRIEVAL_HOURS_MIN && v <= RETRIEVAL_HOURS_MAX;
}

/** "YYYY-MM-DDTHH:MM"（datetime-local）或 "YYYY-MM-DD HH:MM:SS"（后端）→ 本机时刻毫秒；坏值 NaN */
function parseMomentMs(s: string): number {
  const m = s.trim().match(/^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/);
  if (m === null) return NaN;
  return new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]).getTime();
}

/**
 * 喂食提交后的撤食反馈判定：
 * - 所选不含易腐 → none（现状：直接关窗）。
 * - 含易腐但有无效间隔（脏数据，正常被设置必填堵住）→ none。
 * - 全部有效且 发生时刻+min(间隔) > 现在 → fresh（X = 最短间隔小时数）。
 * - 已到期（含恰好到期，严格大于才算新鲜）→ overdue。
 */
export function retrievalFeedback(
  selected: readonly PerishableProbe[],
  happenedAt: string,
  now: string,
): RetrievalFeedback {
  const perishable = selected.filter((f) => f.perishable === true);
  if (perishable.length === 0) return { kind: "none", hours: 0 };
  if (!perishable.every((f) => validRetrievalHours(f.retrieval_hours))) return { kind: "none", hours: 0 };
  const hours = Math.min(...perishable.map((f) => f.retrieval_hours as number));
  const due = parseMomentMs(happenedAt);
  const at = parseMomentMs(now);
  if (Number.isNaN(due) || Number.isNaN(at)) return { kind: "none", hours: 0 };
  return due + hours * MS_PER_HOUR > at ? { kind: "fresh", hours } : { kind: "overdue", hours };
}

/** 反馈句（kind=none → 空串，调用方隐藏该句） */
export function retrievalFeedbackText(fb: RetrievalFeedback): string {
  if (fb.kind === "fresh") return `将于 ${fb.hours} 小时后提醒撤食`;
  if (fb.kind === "overdue") return "已逾期，明起每日提醒撤食";
  return "";
}

/** 食物 chip 悬停说明（易腐项）；非易腐 null（不渲染 title） */
export function perishableChipTitle(f: PerishableProbe): string | null {
  if (f.perishable !== true) return null;
  return validRetrievalHours(f.retrieval_hours)
    ? `易腐 · 撤食间隔 ${f.retrieval_hours} 小时`
    : "易腐 · 未设撤食间隔";
}
</script>

<script setup lang="ts">
/**
 * 喂食弹窗：食物多选 chips（仅启用食物）+ 时间（默认现在、可补录）+ 备注（可选）。
 * 本组件自持状态、自己发 IPC（list_foods / log_care），成功后抛 saved 让外层关窗刷新；
 * 停用食物不进新建入口（规则 10）。
 * 交互第三轮：换 DateTimeField（标记日历：橙点=当前操作/灰点=其它/悬停明细）+
 * 选中日已有同操作记录出黄条（不拦提交）。接入与 QuickLogDialog 同构。
 * 票 04：易腐项加圆点记号；提交成功且有撤食反馈时行内告知、点「知道了」再关窗刷新
 * （无反馈维持现状直接关窗，父层 onFeedSaved 负责刷新）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { CareActionItem, Colony, ColonyAction, FoodItem, MonthDayRecords } from "../types";
import { nowLocalDateTime } from "../lib/care";
import { todayIso } from "../lib/dates";
import { buildMarkers, duplicateInfo, dupWarningText } from "../lib/monthview";
import DateTimeField from "./DateTimeField.vue";

const props = defineProps<{
  colony: Colony;
  action: ColonyAction;
}>();
const emit = defineEmits<{ close: []; saved: [] }>();

const foods = ref<FoodItem[]>([]);
const selectedIds = ref<number[]>([]);
const time = ref(nowLocalDateTime());
const note = ref("");
const formError = ref("");
const busy = ref(false);

const enabledFoods = computed(() => foods.value.filter((f) => f.enabled));

onMounted(async () => {
  try {
    foods.value = await invoke<FoodItem[]>("list_foods");
  } catch (e) {
    formError.value = String(e);
  }
});

const monthRows = ref<MonthDayRecords[]>([]);
const viewMonth = ref({ year: +time.value.slice(0, 4), month: +time.value.slice(5, 7) });
let monthSeq = 0; // 复审 #10：快速翻月旧响应后到会污染标记/黄条，序号守卫丢弃过期响应

async function loadMonth(y: number, m: number) {
  viewMonth.value = { year: y, month: m };
  const seq = ++monthSeq;
  try {
    const res = await invoke<MonthDayRecords[]>("colony_month_records", {
      colonyId: props.colony.id,
      year: y,
      month: m,
    });
    if (seq !== monthSeq) return; // 过期响应，丢弃
    monthRows.value = res;
  } catch {
    if (seq === monthSeq) monthRows.value = []; // 标记是增强，失败静默（提交校验权威在后端）
  }
}
onMounted(() => void loadMonth(viewMonth.value.year, viewMonth.value.month));

// 终局评审：名字表口径统一——colony.actions 只有启用项，停用操作的标记名会丢；
// 改拉 list_actions 全量（含停用），与 LogListPage 编辑弹窗一致
const allActions = ref<CareActionItem[]>([]);
async function loadActions() {
  try {
    allActions.value = await invoke<CareActionItem[]>("list_actions");
  } catch {
    // 名字表是增强，失败静默（当前操作名有 props 兜底）
  }
}
onMounted(() => void loadActions());

// 复审 #15/#16：「现在/±10分/选日期」可跨月，月份跟随时间值重同步（黄条判定依赖 viewMonth）
watch(
  () => time.value.slice(0, 7),
  (ym, old) => {
    if (ym !== old) void loadMonth(+ym.slice(0, 4), +ym.slice(5, 7));
  },
);

/** DatePickerPop/DateTimeField 的 month 事件 → 拉该月数据（显式类型，模板内联箭头在 vue-tsc 下推断不稳） */
function onMonth(view: { year: number; month: number }) {
  void loadMonth(view.year, view.month);
}

/** 日历标记：当前操作橙点、其它操作灰点——名字表必须全量（list_actions，含停用操作），
 * 否则 buildMarkers 跳过缺名操作、灰点与 tip 整体失效（复审 #1/#2；口径与编辑弹窗一致） */
const markers = computed(() => {
  const names = new Map<number, string>(allActions.value.map((a) => [a.id, a.name]));
  names.set(props.action.action_id, props.action.name); // 兜底：当前操作不在字典里也不丢橙点
  return buildMarkers(monthRows.value, props.action.action_id, names);
});

/** 选中日期重复判定（时间字段前 10 位 = 日期） */
const dup = computed(() =>
  duplicateInfo(monthRows.value, viewMonth.value.year, viewMonth.value.month, time.value.slice(0, 10), props.action.action_id),
);
const dupText = computed(() => dupWarningText(dup.value, time.value.slice(0, 10), todayIso(), props.action.name));

function toggleFood(id: number) {
  selectedIds.value = selectedIds.value.includes(id)
    ? selectedIds.value.filter((x) => x !== id)
    : [...selectedIds.value, id];
}

// ── 票 04：提交成功反馈 ──
const savedOk = ref(false);
const savedFeedback = ref<RetrievalFeedback>({ kind: "none", hours: 0 });
const savedFeedbackText = computed(() => retrievalFeedbackText(savedFeedback.value));

/** 本餐所选食物（反馈判定只看它） */
const selectedFoods = computed(() => foods.value.filter((f) => selectedIds.value.includes(f.id)));

/** 有撤食反馈句 → 留在弹窗展示，「知道了」再抛 saved 关窗刷新；否则维持现状直接关窗 */
function afterSaved() {
  const fb = retrievalFeedback(selectedFoods.value, time.value, nowLocalDateTime());
  if (retrievalFeedbackText(fb) === "") {
    emit("saved");
    return;
  }
  savedFeedback.value = fb;
  savedOk.value = true;
}

function finishSaved() {
  savedOk.value = false;
  emit("saved");
}

/** 已展示成功反馈后，点遮罩同样走「知道了」收尾（保证父层刷新，不留脏弹窗） */
function onOverlaySelf() {
  if (savedOk.value) finishSaved();
  else emit("close");
}

async function submit() {
  if (selectedIds.value.length === 0) {
    formError.value = "先选至少一种食物";
    return;
  }
  busy.value = true;
  formError.value = "";
  try {
    await invoke("log_care", {
      input: {
        colony_id: props.colony.id,
        action_id: props.action.action_id,
        happened_at: time.value,
        note: note.value.trim() === "" ? null : note.value.trim(),
        food_ids: selectedIds.value,
      },
    });
    afterSaved();
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="overlay" @click.self="onOverlaySelf">
    <div class="dialog feed-dialog">
      <h3>记录{{ action.name }} · {{ colony.name }}</h3>

      <div class="field-label">食物（可多选）</div>
      <div class="foods">
        <button
          v-for="f in enabledFoods"
          :key="f.id"
          class="food"
          :class="{ selected: selectedIds.includes(f.id), perishable: f.perishable === true }"
          :title="perishableChipTitle(f) ?? undefined"
          type="button"
          @click="toggleFood(f.id)"
        >
          {{ f.name }}<span v-if="f.perishable === true" class="p-dot" aria-hidden="true"></span>
        </button>
      </div>

      <div class="field-label">时间（默认现在，可补录）</div>
      <DateTimeField v-model="time" :markers="markers" @month="onMonth" />
      <p v-if="dupText !== ''" class="dup-warn">⚠ {{ dupText }}<span class="why">确属再次操作可直接记录</span></p>

      <div class="field-label">备注（可选）</div>
      <textarea v-model="note" class="note-input" placeholder="如：换了新一批面包虫"></textarea>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <!-- 票 04：撤食反馈只在有话可说时出现（fresh/overdue），none 分支维持直接关窗 -->
      <div v-if="savedOk" class="save-ok">
        <p class="ok-line">✓ 已记录</p>
        <p class="fb-line">{{ savedFeedbackText }}</p>
      </div>

      <div class="dlg-btns">
        <button v-if="!savedOk" class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button v-if="savedOk" class="btn primary record-btn" type="button" @click="finishSaved">知道了</button>
        <button v-else class="btn primary record-btn" type="button" :disabled="busy" @click="submit">
          记录
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 视觉基线 mocks/mock-a-light.html 喂食弹窗 */
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
  width: 390px;
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

/* 易腐记号（票 04）：chip 内小圆点，色同日历当前操作点（--accent 橙），悬停说明在 title */
.food .p-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent, #d97706);
  margin-left: 6px;
  vertical-align: middle;
}

.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

/* 黄条：当天已有同操作记录提醒（不拦提交，视觉基线 mocks/mock-c-calendar.html） */
.dup-warn {
  margin-top: 6px; padding: 7px 10px; border-radius: 9px;
  background: #fdf1de; border: 1px solid #f3d9a8; color: #b45309;
  font-size: 12px; display: flex; gap: 6px; align-items: baseline;
}
.dup-warn .why { margin-left: auto; font-size: 11px; opacity: 0.8; white-space: nowrap; }

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

/* 提交成功反馈（票 04）：已记录 + 撤食安排句（fresh/overdue 文案在组件里拼好） */
.save-ok {
  margin-top: 10px;
  padding: 7px 10px;
  border-radius: 9px;
  background: var(--ok-soft);
  font-size: 12px;
}

.save-ok .ok-line {
  color: var(--ok);
  font-weight: 600;
}

.save-ok .fb-line {
  color: var(--text);
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
