<script setup lang="ts">
/**
 * 快捷打卡面板（反馈第二轮 F1，Q1=C）：非喂食操作点击后先弹此面板——
 * 日期默认今天、可补录过去、可填备注，点「记录」才落库；取消不产生任何记录。
 * 喂食不弹这里（FeedDialog 自带食物多选）。后端 log_care 已拒未来时间。
 * 交互第三轮：换 DateTimeField（标记日历：橙点=当前操作/灰点=其它/悬停明细）+
 * 选中日已有同操作记录出黄条（不拦提交）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyAction, MonthDayRecords } from "../types";
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

/** 日历标记：当前操作橙点、其它操作灰点——名字表必须全量（colony.actions），
 * 否则 buildMarkers 跳过缺名操作、灰点与 tip 整体失效（复审 #1/#2） */
const markers = computed(() => {
  const names = new Map<number, string>(props.colony.actions.map((a) => [a.action_id, a.name]));
  names.set(props.action.action_id, props.action.name); // 兜底：当前操作不在 actions 里也不丢橙点
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
    await invoke("log_care", {
      input: {
        colony_id: props.colony.id,
        action_id: props.action.action_id,
        happened_at: time.value,
        note: note.value.trim() === "" ? null : note.value.trim(),
        food_ids: [],
      },
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
    <div class="dialog quick-dialog">
      <h3>记录{{ action.name }} · {{ colony.name }}</h3>

      <div class="field-label">时间（默认现在，可补录）</div>
      <DateTimeField v-model="time" :markers="markers" @month="onMonth" />
      <p v-if="dupText !== ''" class="dup-warn">⚠ {{ dupText }}<span class="why">确属再次操作可直接记录</span></p>

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
