<script setup lang="ts">
/**
 * 日期+时间选择（交互第三轮 #4）：DatePickerPop 管日期，时间行给
 * 「现在/−10分/＋10分」快捷键与时分步进（mocks/mock-c-calendar.html ①区）。
 * modelValue 与原 input[type=datetime-local] 同形态（YYYY-MM-DDTHH:mm），
 * 替换点对弹窗载荷/校验零侵入。空值选日期时时间兜底 "00:00"（复审修正）。
 */
import { computed } from "vue";
import DatePickerPop from "./DatePickerPop.vue";
import { shiftMinutes, stepTimeField, weekdayShort } from "../lib/calendar";
import { nowLocalDateTime } from "../lib/care";

const props = withDefaults(
  defineProps<{ modelValue: string; markers?: Record<string, { current: boolean; tip: string }> }>(),
  { markers: () => ({}) },
);
const emit = defineEmits<{ "update:modelValue": [value: string]; month: [view: { year: number; month: number }] }>();

const datePart = computed(() => props.modelValue.slice(0, 10));
const timePart = computed(() => props.modelValue.slice(11, 16));

function set(value: string) {
  emit("update:modelValue", value);
}
/** 选日期与原时间合成；原值为空（或只有日期）时时间兜底 "00:00"（复审修正） */
function pickDate(iso: string) {
  set(`${iso}T${timePart.value === "" ? "00:00" : timePart.value}`);
}
function shift(delta: number) {
  set(shiftMinutes(props.modelValue, delta));
}
function step(field: "h" | "m", delta: number) {
  set(stepTimeField(props.modelValue, field, delta));
}
/** DatePickerPop 的 month 事件原样转发（显式类型，模板内联箭头在 vue-tsc 下推断不稳） */
function onMonth(view: { year: number; month: number }) {
  emit("month", view);
}
</script>

<template>
  <div class="dtf">
    <DatePickerPop
      :model-value="datePart"
      :markers="markers"
      placeholder="选择日期"
      @update:model-value="pickDate"
      @month="onMonth"
    />
    <div class="dtf-time-row">
      <button class="dtf-chip dtf-now" type="button" @click="set(nowLocalDateTime())">现在</button>
      <button class="dtf-chip dtf-m10" type="button" @click="shift(-10)">−10分</button>
      <button class="dtf-chip dtf-p10" type="button" @click="shift(10)">＋10分</button>
      <span class="dtf-time">
        <span class="seg">
          <button data-step="h-" type="button" @click="step('h', -1)">‹</button>
          <span class="num">{{ timePart.slice(0, 2) }}</span>
          <button data-step="h+" type="button" @click="step('h', 1)">›</button>
        </span>
        <span class="sep">:</span>
        <span class="seg">
          <button data-step="m-" type="button" @click="step('m', -1)">‹</button>
          <span class="num">{{ timePart.slice(3, 5) }}</span>
          <button data-step="m+" type="button" @click="step('m', 1)">›</button>
        </span>
      </span>
      <span v-if="datePart !== ''" class="dtf-wd">周{{ weekdayShort(datePart) }}</span>
    </div>
  </div>
</template>

<style scoped>
.dtf { display: flex; flex-direction: column; gap: 8px; }
.dtf-time-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.dtf-chip {
  border: 1px solid var(--border-strong, #d8d1c4); background: var(--tile, #faf8f5);
  border-radius: 999px; padding: 3px 11px; font: inherit; font-size: 12px;
  color: var(--text, #2c2822); cursor: pointer;
}
.dtf-chip:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dtf-time { display: flex; align-items: center; gap: 3px; margin-left: 4px; font-variant-numeric: tabular-nums; }
.dtf-time .seg { display: flex; align-items: center; gap: 2px; }
.dtf-time .num { font-size: 15px; font-weight: 700; min-width: 26px; text-align: center; }
.dtf-time .sep { font-weight: 700; }
.dtf-time button {
  border: 1px solid var(--border, #e8e2d8); background: var(--tile, #faf8f5); border-radius: 6px;
  cursor: pointer; font: inherit; font-size: 10px; padding: 1px 5px; color: var(--muted, #8f887d); line-height: 1.3;
}
.dtf-time button:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dtf-wd { font-size: 12px; color: var(--muted, #8f887d); }
</style>
