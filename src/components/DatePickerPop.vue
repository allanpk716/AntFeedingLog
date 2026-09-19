<script setup lang="ts">
/**
 * 自绘日历弹层（交互第三轮 #3，视觉基线 mocks/mock-c-calendar.html）：
 * 选中即关、翻月/今天、Esc 与点外部关闭；弹层几何（复审修正）：垂直向空间更大的一侧
 * 展开、水平双向钳制（右缘放不下整体左移，且不被推出左缘），happy-dom/异形窗口下不超窗。
 * markers 传入即带记录标记层（当前操作橙点/其它灰点/悬停 tip），不传即素版。
 * 纯日期版；datetime 场景由 DateTimeField 组合本组件 + 时间快捷行。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { monthGrid } from "../lib/calendar";
import { todayIso } from "../lib/dates";

const props = withDefaults(
  defineProps<{
    modelValue: string;
    placeholder?: string;
    markers?: Record<string, { current: boolean; tip: string }>;
  }>(),
  { placeholder: "选择日期", markers: () => ({}) },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
  month: [view: { year: number; month: number }];
}>();

/** 当前展示月（初始 = 选中值或今天所在月） */
const today = todayIso();
const initial = props.modelValue !== "" ? props.modelValue : today;
const view = ref({ year: +initial.slice(0, 4), month: +initial.slice(5, 7) });

const open = ref(false);
const flipUp = ref(false);
const triggerRef = ref<HTMLElement | null>(null);

const cells = computed(() => monthGrid(view.value.year, view.value.month));
const label = computed(() => `${view.value.year}年${view.value.month}月`);
const display = computed(() =>
  props.modelValue === "" ? props.placeholder : props.modelValue,
);

/** 展示月变化（含翻页/今天跳转）都通知父级，供其拉该月标记数据 */
function notifyMonth() {
  emit("month", { ...view.value });
}
onMounted(notifyMonth);

function nav(delta: number) {
  let m = view.value.month + delta;
  let y = view.value.year;
  if (m < 1) { m = 12; y -= 1; }
  if (m > 12) { m = 1; y += 1; }
  view.value = { year: y, month: m };
  notifyMonth();
}

function goToday() {
  view.value = { year: +today.slice(0, 4), month: +today.slice(5, 7) };
  notifyMonth();
}

/** 弹层固定宽（px）与距窗缘安全边距 */
const POP_W = 252;
const EDGE = 12;
const popShift = ref(0);
function toggle() {
  if (open.value) { open.value = false; return; }
  const rect = triggerRef.value?.getBoundingClientRect();
  if (rect !== undefined) {
    // 复审修正：垂直向空间更大的一侧展开（不再要求上方>320px 才翻）
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    flipUp.value = spaceAbove > spaceBelow;
    // 复审修正：水平双向钳制——右缘放不下整体左移，但钳住不被推出左缘
    const rightClamp = Math.min(0, window.innerWidth - EDGE - POP_W - rect.left);
    popShift.value = Math.max(rightClamp, EDGE - rect.left);
  }
  open.value = true;
}

function pick(iso: string) {
  open.value = false;                 // 选中即关（mock C4 定稿行为）
  emit("update:modelValue", iso);
}

function onDocClick(e: MouseEvent) {
  if (!open.value) return;
  const root = triggerRef.value?.parentElement;
  if (root !== null && root !== undefined && e.target instanceof Node && root.contains(e.target)) return;
  open.value = false;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") open.value = false;
}
onMounted(() => {
  document.addEventListener("click", onDocClick);
  document.addEventListener("keydown", onKey);
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
  document.removeEventListener("keydown", onKey);
});

watch(
  () => props.modelValue,
  (v) => {
    if (v !== "") view.value = { year: +v.slice(0, 4), month: +v.slice(5, 7) };
  },
);
</script>

<template>
  <span class="dp">
    <button ref="triggerRef" class="dp-trigger" :class="{ empty: modelValue === '' }" type="button" @click="toggle">
      {{ display }}<span class="dp-caret">▾</span>
    </button>
    <div v-if="open" class="dp-pop" :class="{ up: flipUp }" :style="{ left: popShift + 'px' }">
      <div class="dp-head">
        <button class="dp-nav dp-prev" type="button" @click="nav(-1)">‹</button>
        <span class="dp-title">{{ label }}</span>
        <button class="dp-nav dp-next" type="button" @click="nav(1)">›</button>
        <button class="dp-today" type="button" @click="goToday">今天</button>
      </div>
      <div class="dp-grid">
        <span v-for="w in '一二三四五六日'" :key="w" class="dp-wd">{{ w }}</span>
        <template v-for="(c, i) in cells" :key="i">
          <span v-if="c.iso === null" class="dp-day dim"></span>
          <button
            v-else
            class="dp-day"
            :class="{ sel: c.iso === modelValue, tod: c.iso === today }"
            :title="markers[c.iso]?.tip"
            type="button"
            @click="pick(c.iso)"
          >
            {{ +c.iso.slice(8, 10) }}
            <span v-if="markers[c.iso]" class="dp-dots">
              <span v-if="markers[c.iso]!.current" class="dot cur"></span>
              <span v-else class="dot"></span>
            </span>
          </button>
        </template>
      </div>
      <div v-if="Object.keys(markers).length > 0" class="dp-legend">
        <span class="k"><span class="dot cur"></span>当天已有当前操作</span>
        <span class="k"><span class="dot"></span>其它操作（悬停看明细）</span>
      </div>
      <div class="dp-foot">选中即关闭 · Esc / 点外部关闭</div>
    </div>
  </span>
</template>

<style scoped>
.dp { position: relative; display: inline-block; }
.dp-trigger {
  padding: 6px 10px; border: 1px solid var(--border-strong, #d8d1c4); border-radius: 8px;
  font: inherit; font-size: 13px; background: var(--card, #fff); color: var(--text, #2c2822);
  cursor: pointer; min-width: 104px; text-align: left;
}
.dp-trigger:hover { border-color: var(--accent, #d97706); }
.dp-trigger.empty { color: var(--muted, #8f887d); }
.dp-caret { float: right; color: var(--muted, #8f887d); margin-left: 6px; }

.dp-pop {
  position: absolute; top: calc(100% + 6px); left: 0; z-index: 60;
  width: 252px; background: var(--card, #fff); border: 1px solid var(--border-strong, #d8d1c4);
  border-radius: 12px; box-shadow: 0 10px 40px rgba(60, 50, 30, 0.2); padding: 10px 12px;
}
.dp-pop.up { top: auto; bottom: calc(100% + 6px); }
.dp-head { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; }
.dp-title { font-size: 13px; font-weight: 700; flex: 1; text-align: center; }
.dp-nav, .dp-today {
  border: 1px solid var(--border, #e8e2d8); background: var(--tile, #faf8f5); border-radius: 7px;
  cursor: pointer; font: inherit; font-size: 12px; padding: 1px 8px; color: var(--text, #2c2822);
}
.dp-today { border-radius: 999px; font-size: 11px; color: var(--muted, #8f887d); }
.dp-nav:hover, .dp-today:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dp-grid { display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; text-align: center; }
.dp-wd { font-size: 10px; color: var(--muted, #8f887d); padding: 2px 0; }
.dp-day {
  position: relative; border: none; background: transparent; font: inherit; font-size: 12px;
  color: var(--text, #2c2822); border-radius: 8px; padding: 3px 0 9px; cursor: pointer; min-width: 28px;
}
.dp-day:hover { background: var(--accent-soft, #fdf1de); }
.dp-day.dim { cursor: default; }
.dp-day.dim:hover { background: transparent; }
.dp-day.sel { background: var(--accent, #d97706); color: #fff; font-weight: 700; }
.dp-day.tod:not(.sel) { box-shadow: inset 0 0 0 1.5px var(--accent, #d97706); color: var(--accent-deep, #b45309); font-weight: 600; }
.dp-dots { position: absolute; left: 0; right: 0; bottom: 2px; display: flex; justify-content: center; height: 5px; }
.dot { width: 5px; height: 5px; border-radius: 50%; background: #c9c1b2; }
.dot.cur { width: 7px; height: 7px; background: var(--accent, #d97706); box-shadow: 0 0 0 2px var(--accent-soft, #fdf1de); }
.dp-day.sel .dot { background: rgba(255, 255, 255, 0.75); }
.dp-day.sel .dot.cur { background: #fff; box-shadow: none; }
.dp-legend {
  margin-top: 7px; padding-top: 7px; border-top: 1px dashed var(--border, #e8e2d8);
  font-size: 10px; color: var(--muted, #8f887d); display: flex; gap: 10px; justify-content: center; flex-wrap: wrap;
}
.dp-legend .k { display: flex; align-items: center; gap: 4px; }
.dp-legend .dot { position: static; }
.dp-foot { margin-top: 4px; font-size: 10px; color: var(--muted, #8f887d); text-align: center; }
</style>
