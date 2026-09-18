<script setup lang="ts">
/**
 * 统计页（票 07）：筛选（窝下拉 / 时间范围）+ 四块——日历热力图（ECharts
 * calendar，悬停看当天明细含食物）、喂食构成环形图（归一化 100%，图例带次数）、
 * 每周操作柱状（近 12 周）、各操作实际间隔 vs 建议间隔（手写条，仅提醒类画
 * 建议刻度竖线，登记类标「仅登记」）。
 * 视觉基线 mocks/mock-b-stats.html；占比归一化/周取数/间隔条布局在 lib/stats.ts。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import * as echarts from "echarts";
import type { Colony, StatsDayDetail, StatsPayload } from "../types";
import { todayIso } from "../lib/dates";
import {
  avgPerDay,
  buildDetailMap,
  dayTooltip,
  intervalRows,
  lastWeeks,
  normalizeFoodShare,
  rangeStartFor,
  shortWeek,
  type StatsRange,
} from "../lib/stats";

// 视觉基线取色（mock-b）
const MUTED = "#8f887d";
const BORDER = "#e8e2d8";
const HEAT_COLORS = ["#ece8e1", "#f5d9a4", "#e9b45f", "#d68a2a", "#a34f06"];
const BAR_COLOR = "#d68a2a";
const PALETTE = ["#d97706", "#4a90d9", "#8b5e34", "#188a4b", "#a34f06", "#5b6472", "#c258a0", "#6a8f3c"];
const MONTHS = ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"];

const today = todayIso();
const colonies = ref<Colony[]>([]);
const colonyId = ref<number | null>(null);
const range = ref<StatsRange>("6m");
const payload = ref<StatsPayload | null>(null);
const pageError = ref("");
const loading = ref(false);

const totalLogs = computed(() => payload.value?.daily.reduce((s, d) => s + d.count, 0) ?? 0);
const foodSlices = computed(() => normalizeFoodShare(payload.value?.food_share ?? []));
const feedTotalCount = computed(() =>
  payload.value?.food_share.reduce((s, x) => s + x.occurrences, 0) ?? 0,
);
const bars = computed(() => lastWeeks(payload.value?.weekly ?? [], 12));
const intervalList = computed(() => intervalRows(payload.value?.intervals ?? []));

async function refresh() {
  loading.value = true;
  try {
    if (colonies.value.length === 0) {
      colonies.value = await invoke<Colony[]>("list_colonies");
    }
    // 「全部」下界 = min(最早开始饲养日, 最早记录日)（票 07 停靠①）；分母口径
    // （规则 7）由 range_days 随 payload 带回
    const earliest = await invoke<string | null>("earliest_log_date");
    payload.value = await invoke<StatsPayload>("get_stats", {
      colonyId: colonyId.value,
      startDate: rangeStartFor(range.value, today, colonies.value, earliest),
      endDate: today,
    });
    pageError.value = "";
  } catch (e) {
    pageError.value = String(e);
  } finally {
    loading.value = false;
  }
}

function onColonyChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  colonyId.value = v === "" ? null : Number(v);
  void refresh();
}

function onRangeChange(e: Event) {
  range.value = (e.target as HTMLSelectElement).value as StatsRange;
  void refresh();
}

// ── 图表（ECharts；SVG 渲染，显式尺寸，页签卸载即释放）──────────────────

const heatEl = ref<HTMLDivElement | null>(null);
const donutEl = ref<HTMLDivElement | null>(null);
const weeklyEl = ref<HTMLDivElement | null>(null);
let charts: echarts.ECharts[] = [];

function disposeCharts() {
  for (const c of charts) c.dispose();
  charts = [];
}

function renderCharts() {
  const p = payload.value;
  if (!p) return;
  disposeCharts();
  renderHeat(buildDetailMap(p.daily_detail));
  renderDonut();
  renderWeekly();
}

function renderHeat(detailMap: Map<string, StatsDayDetail["entries"]>) {
  const el = heatEl.value;
  const p = payload.value;
  if (!el || !p) return;
  const weeks = Math.ceil(p.range_days / 7);
  const width = Math.max(30 + weeks * 16, 240);
  const chart = echarts.init(el, undefined, { renderer: "svg", width, height: 150 });
  charts.push(chart);
  chart.setOption({
    tooltip: {
      formatter: (param: unknown) => {
        const value = (param as { value: [string, number] }).value;
        return dayTooltip(value[0], detailMap);
      },
    },
    visualMap: {
      show: false,
      min: 0,
      max: Math.max(4, ...p.daily.map((d) => d.count)),
      inRange: { color: HEAT_COLORS },
    },
    calendar: {
      range: [p.range_start, p.range_end],
      cellSize: [13, 13],
      left: 26,
      top: 22,
      splitLine: { show: false },
      itemStyle: { color: "#ece8e1", borderWidth: 2, borderColor: "#ffffff" },
      dayLabel: { firstDay: 1, nameMap: "cn", fontSize: 10, color: MUTED },
      monthLabel: { nameMap: MONTHS, fontSize: 11, color: MUTED },
      yearLabel: { show: false },
    },
    series: [
      {
        type: "heatmap",
        coordinateSystem: "calendar",
        data: p.daily.map((d) => [d.date, d.count]),
      },
    ],
  });
}

function renderDonut() {
  const el = donutEl.value;
  if (!el || foodSlices.value.length === 0) return;
  const chart = echarts.init(el, undefined, { renderer: "svg", width: 170, height: 150 });
  charts.push(chart);
  chart.setOption({
    tooltip: {
      formatter: (param: unknown) => {
        const q = param as { name: string; value: number; percent: number };
        return `${q.name}：${q.value} 次（${q.percent}%）`;
      },
    },
    series: [
      {
        type: "pie",
        radius: ["58%", "82%"],
        label: { show: false },
        labelLine: { show: false },
        data: foodSlices.value.map((s, i) => ({
          name: s.food_name,
          value: s.occurrences,
          itemStyle: { color: PALETTE[i % PALETTE.length] },
        })),
      },
    ],
  });
}

function renderWeekly() {
  const el = weeklyEl.value;
  const p = payload.value;
  if (!el || !p) return;
  const chart = echarts.init(el, undefined, { renderer: "svg", width: 460, height: 180 });
  charts.push(chart);
  chart.setOption({
    grid: { left: 34, right: 10, top: 24, bottom: 26 },
    tooltip: {},
    xAxis: {
      type: "category",
      data: bars.value.map((w) => shortWeek(w.week_start)),
      axisLabel: { interval: 1, fontSize: 10, color: MUTED },
      axisLine: { lineStyle: { color: BORDER } },
      axisTick: { show: false },
    },
    yAxis: {
      type: "value",
      minInterval: 1,
      axisLabel: { fontSize: 10, color: MUTED },
      splitLine: { lineStyle: { color: BORDER } },
    },
    series: [
      {
        type: "bar",
        data: bars.value.map((w) => w.count),
        barMaxWidth: 38,
        itemStyle: { color: BAR_COLOR, borderRadius: [4, 4, 0, 0] },
        label: { show: true, position: "top", fontSize: 10, color: MUTED },
      },
    ],
  });
}

onMounted(() => {
  void refresh();
});
watch(payload, () => void nextTick(renderCharts));
onBeforeUnmount(disposeCharts);
</script>

<template>
  <div class="stats-page">
    <div class="filters">
      <label class="f-label">窝</label>
      <select class="colony-select" :value="colonyId ?? ''" @change="onColonyChange">
        <option value="">全部</option>
        <option v-for="c in colonies" :key="c.id" :value="c.id">{{ c.name }}</option>
      </select>
      <label class="f-label">时间范围</label>
      <select class="range-select" :value="range" @change="onRangeChange">
        <option value="6m">近 6 个月</option>
        <option value="3m">近 3 个月</option>
        <option value="all">全部</option>
      </select>
    </div>

    <p v-if="pageError" class="page-error">{{ pageError }}</p>

    <div class="kpis">
      <div class="kpi">
        <div class="n">{{ totalLogs }}<small> 条</small></div>
        <div class="l">记录总数</div>
      </div>
      <div class="kpi">
        <div class="n">{{ avgPerDay(totalLogs, payload?.range_days ?? 0) }}<small> 次/天</small></div>
        <div class="l">平均每天操作</div>
      </div>
    </div>
    <p class="freq-note" data-testid="freq-note">
      口径：平均每天 = 记录总数 ÷ {{ payload?.range_days ?? 0 }} 个自然日（不扣冬眠）。
    </p>

    <p v-if="payload && totalLogs === 0" class="empty-hint">该范围内暂无记录</p>

    <section class="card">
      <h3>日历热力图 <span class="hint">一格一天，颜色越深当天操作越多 · 悬停看明细</span></h3>
      <div class="heat-area">
        <div ref="heatEl" class="heat-chart"></div>
        <div class="legend">
          少
          <i v-for="c in HEAT_COLORS" :key="c" :style="{ background: c }"></i>
          多
        </div>
      </div>
    </section>

    <div class="row2">
      <section class="card">
        <h3>喂食构成 <span class="hint">按食物出现次数归一化</span></h3>
        <div class="donut-wrap">
          <div v-if="foodSlices.length > 0" class="donut-box">
            <div ref="donutEl" class="donut"></div>
            <div class="donut-center">
              <b>{{ feedTotalCount }}</b>
              <span>次投喂</span>
            </div>
          </div>
          <ul v-if="foodSlices.length > 0" class="dlegend">
            <li v-for="(s, i) in foodSlices" :key="s.food_name">
              <span class="dot" :style="{ background: PALETTE[i % PALETTE.length] }"></span>
              <span>{{ s.food_name }}</span>
              <b class="pct">{{ s.pct }}%</b>
              <span class="lc">{{ s.occurrences }} 次</span>
            </li>
          </ul>
          <div v-else class="dlegend"><p class="empty">暂无喂食记录</p></div>
        </div>
      </section>

      <section class="card">
        <h3>每周操作次数 <span class="hint">最近 12 周</span></h3>
        <div ref="weeklyEl" class="weekly-chart"></div>
      </section>
    </div>

    <section class="card">
      <h3>实际间隔 vs 建议间隔 <span class="hint">间隔已扣除冬眠天数 · 竖线 = 建议间隔（仅提醒类）</span></h3>
      <div>
        <div v-for="row in intervalList" :key="row.action_id" class="irow">
          <span class="iname">{{ row.name }}</span>
          <template v-if="row.avgPct === null">
            <div class="itrack"></div>
            <span class="itext">记录不足 · {{ row.tail }}</span>
          </template>
          <template v-else>
            <div class="itrack">
              <div class="ibar" :style="{ width: row.avgPct + '%' }"></div>
              <div v-if="row.markPct !== null" class="imark" :style="{ left: row.markPct + '%' }"></div>
            </div>
            <span class="itext">
              平均 <b>{{ row.avgLabel }}</b> 天 · 最短 {{ row.min_days }} · 最长 {{ row.max_days }} · {{ row.tail }}
            </span>
          </template>
        </div>
        <p v-if="intervalList.length === 0" class="empty">暂无操作可统计</p>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* 视觉基线 mocks/mock-b-stats.html，取色与 mock-a 一致 */
.stats-page {
  --card: #ffffff;
  --tile: #faf8f5;
  --text: #2c2822;
  --muted: #8f887d;
  --border: #e8e2d8;
  --border-strong: #d8d1c4;
  --accent-deep: #b45309;
  --accent-soft: #fdf1de;
  --bad: #d13d3d;
  --bad-soft: #fcebeb;
  --shadow: 0 1px 2px rgba(60, 50, 30, 0.05), 0 4px 14px rgba(60, 50, 30, 0.06);
}

.filters {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 20px 0 4px;
  flex-wrap: wrap;
}

.f-label {
  font-size: 13px;
  color: var(--muted);
}

select {
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 9px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.page-error {
  margin-top: 16px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--bad-soft);
  color: var(--bad);
  font-size: 13px;
}

.kpis {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 14px;
  margin-top: 14px;
}

.kpi {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 14px 16px;
  box-shadow: var(--shadow);
}

.kpi .n {
  font-size: 26px;
  font-weight: 800;
}

.kpi .n small {
  font-size: 13px;
  font-weight: 600;
  color: var(--muted);
}

.kpi .l {
  font-size: 12px;
  color: var(--muted);
  margin-top: 2px;
}

.freq-note {
  font-size: 12px;
  color: var(--muted);
  margin-top: 8px;
}

.empty-hint {
  margin-top: 14px;
  padding: 10px 14px;
  border: 1px dashed var(--border-strong);
  border-radius: 12px;
  color: var(--muted);
  font-size: 13px;
  text-align: center;
}

.card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 18px;
  box-shadow: var(--shadow);
  margin-top: 14px;
}

.card h3 {
  font-size: 15px;
  display: flex;
  align-items: baseline;
  gap: 10px;
  flex-wrap: wrap;
}

.card h3 .hint {
  font-size: 12px;
  font-weight: 400;
  color: var(--muted);
}

.row2 {
  display: grid;
  grid-template-columns: 1fr 1.6fr;
  gap: 14px;
}

@media (max-width: 820px) {
  .row2 {
    grid-template-columns: 1fr;
  }
}

/* 热力图 */
.heat-area {
  margin-top: 14px;
  overflow-x: auto;
}

.heat-chart {
  min-width: 240px;
}

.legend {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--muted);
  margin-top: 8px;
}

.legend i {
  width: 11px;
  height: 11px;
  border-radius: 3px;
  display: inline-block;
}

/* 喂食构成 */
.donut-wrap {
  display: flex;
  align-items: center;
  gap: 20px;
  margin-top: 16px;
  flex-wrap: wrap;
}

.donut-box {
  position: relative;
  width: 170px;
  height: 150px;
  flex: none;
}

.donut {
  width: 170px;
  height: 150px;
}

.donut-center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  pointer-events: none;
}

.donut-center b {
  font-size: 20px;
}

.donut-center span {
  font-size: 11px;
  color: var(--muted);
}

.dlegend {
  list-style: none;
  font-size: 13px;
  margin: 0;
  padding: 0;
}

.dlegend li {
  margin: 7px 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.dlegend .dot {
  width: 10px;
  height: 10px;
  border-radius: 3px;
  flex: none;
}

.dlegend .pct {
  font-size: 15px;
}

.dlegend .lc {
  color: var(--muted);
  font-size: 12px;
}

.dlegend .empty {
  color: var(--muted);
  font-size: 13px;
}

.weekly-chart {
  margin-top: 16px;
}

/* 间隔条（照 mock-b） */
.irow {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-top: 14px;
  flex-wrap: wrap;
}

.iname {
  width: 110px;
  font-size: 13px;
  flex: none;
}

.itrack {
  flex: 1;
  min-width: 180px;
  height: 12px;
  background: var(--tile);
  border: 1px solid var(--border);
  border-radius: 999px;
  position: relative;
}

.ibar {
  height: 100%;
  background: linear-gradient(90deg, #e9a23f, #c87412);
  border-radius: 999px;
}

.imark {
  position: absolute;
  top: -4px;
  bottom: -4px;
  width: 2px;
  background: var(--accent-deep);
  border-radius: 1px;
}

.imark::after {
  content: "建议";
  position: absolute;
  top: -16px;
  left: -9px;
  font-size: 10px;
  color: var(--accent-deep);
}

.itext {
  font-size: 12px;
  color: var(--muted);
  flex: none;
}

.itext b {
  color: var(--text);
  font-size: 13px;
}
</style>
