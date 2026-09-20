<script setup lang="ts">
/**
 * 快捷打卡面板（反馈第二轮 F1，Q1=C）：非喂食操作点击后先弹此面板——
 * 日期默认今天、可补录过去、可填备注，点「记录」才落库；取消不产生任何记录。
 * 喂食不弹这里（FeedDialog 自带食物多选）。后端 log_care 已拒未来时间。
 * 交互第三轮：换 DateTimeField（标记日历：橙点=当前操作/灰点=其它/悬停明细）+
 * 选中日已有同操作记录出黄条（不拦提交）。
 * 顺带撤食（ADR 0006）：仅 implies_retrieval 操作（预置「垃圾清理」）出行——
 * 已逾期默认勾、未到期默认不勾、无待撤/所选时刻早于易腐喂食不出行；
 * 态随所选时刻动态重算，态变化才重置勾选（用户改过且态没变时不打扰）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { colonyMonthRecords, listActions, logCare, retrievalLinkState } from "../lib/ipc";
import type { RetrievalLinkState } from "../lib/ipc";
import type { CareActionItem, Colony, ColonyAction, MonthDayRecords } from "../types";
import { nowLocalDateTime } from "../lib/care";
import { todayIso } from "../lib/dates";
import { buildMarkers, duplicateInfo, dupWarningText } from "../lib/monthview";
import DateTimeField from "./DateTimeField.vue";

const props = defineProps<{ colony: Colony; action: ColonyAction }>();
const emit = defineEmits<{ close: []; saved: [] }>();

const time = ref(nowLocalDateTime());
const note = ref("");
const formError = ref("");
const busy = ref(false);

const monthRows = ref<MonthDayRecords[]>([]);
const viewMonth = ref({ year: +time.value.slice(0, 4), month: +time.value.slice(5, 7) });
let monthSeq = 0; // 复审 #10：快速翻月旧响应后到会污染标记/黄条，序号守卫丢弃过期响应

async function loadMonth(y: number, m: number) {
  viewMonth.value = { year: y, month: m };
  const seq = ++monthSeq;
  try {
    const res = await colonyMonthRecords({
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
    allActions.value = await listActions();
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

// ── 顺带撤食（ADR 0006）──────────────────────────────────────────────
const linkState = ref<RetrievalLinkState>("none");
const alsoRetrieval = ref(false);
let linkSeq = 0;
let lastApplied: RetrievalLinkState | null = null;

const linkEligible = computed(() => props.action.implies_retrieval === true);
const linkVisible = computed(() => linkEligible.value && linkState.value !== "none");

async function refreshLink() {
  if (!linkEligible.value) return;
  const seq = ++linkSeq;
  try {
    const state = await retrievalLinkState({ colonyId: props.colony.id, at: time.value });
    if (seq !== linkSeq) return; // 过期响应丢弃（快速改时间的竞态）
    linkState.value = state;
    if (state !== lastApplied) {
      // 默认值只在态变化时重置：逾期勾、未到期不勾；态没变时保留用户手选
      alsoRetrieval.value = state === "overdue";
      lastApplied = state;
    }
  } catch {
    if (seq === linkSeq) linkState.value = "none"; // 判定是增强：失败不出行，提交侧后端权威
  }
}
onMounted(() => void refreshLink());
watch(time, () => void refreshLink());

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

async function submit() {
  busy.value = true;
  formError.value = "";
  try {
    await logCare({
      input: {
        colony_id: props.colony.id,
        action_id: props.action.action_id,
        happened_at: time.value,
        note: note.value.trim() === "" ? null : note.value.trim(),
        food_ids: [],
      },
      alsoRetrieval: linkVisible.value && alsoRetrieval.value,
    });
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="overlay" @click.self="$emit('close')">
    <div class="dialog quick-dialog vp-dialog">
      <h3>记录{{ action.name }} · {{ colony.name }}</h3>

      <div class="field-label">时间（默认现在，可补录）</div>
      <DateTimeField v-model="time" :markers="markers" @month="onMonth" />
      <p v-if="dupText !== ''" class="dup-warn">⚠ {{ dupText }}<span class="why">确属再次操作可直接记录</span></p>

      <label v-if="linkVisible" class="link-row" data-testid="retrieval-link">
        <input v-model="alsoRetrieval" type="checkbox" />
        <span>顺带撤走易腐食物</span>
        <span class="link-hint">{{ linkState === "overdue" ? "已到撤食时间" : "未到撤食间隔" }}</span>
      </label>

      <div class="field-label">备注（可选）</div>
      <textarea v-model="note" class="note-input" placeholder="如：顺手检查了垃圾区"></textarea>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <div class="dlg-btns">
        <button class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary record-btn" type="button" :disabled="busy" @click="submit">
          记录
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 视觉基线 mocks/mock-a-light.html 快捷打卡面板（同 FeedDialog 弹窗骨架） */
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

/* 顺带撤食勾选行（ADR 0006）：仅垃圾清理面板、有待撤时出现 */
.link-row {
  margin-top: 10px; padding: 7px 10px; border-radius: 9px;
  background: var(--tile); border: 1px solid var(--border);
  font-size: 13px; display: flex; align-items: center; gap: 8px; cursor: pointer;
}
.link-row input { accent-color: var(--accent); }
.link-hint { margin-left: auto; font-size: 11px; color: var(--muted); white-space: nowrap; }

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

/* ── 手机竖屏（webui-checkin 票 08）：≤480px 输入放大 + 大号按钮；
   vp-dialog 是媒体查询落点（断点类名供组件测试断言——jsdom 不套用媒体查询）── */
@media (max-width: 480px) {
  .vp-dialog.dialog {
    padding: 14px;
  }

  .vp-dialog input[type="datetime-local"],
  .vp-dialog textarea {
    padding: 11px 12px;
    font-size: 16px; /* ≥16px 防 iOS 聚焦自动放大 */
  }

  .vp-dialog .btn {
    padding: 11px 22px;
    font-size: 15px;
  }

  .vp-dialog .dlg-btns {
    flex-direction: row-reverse; /* 主按钮在拇指侧 */
  }
}
</style>
