<script setup lang="ts">
/**
 * 窝卡片：名字、物种徽章、状态徽章、饲养天数大数字（Rust 算好），
 * + 冬眠横幅（票 05：入眠日/预计出眠/剩余天数，预计出眠前 7 天内加「临近出眠」角标）
 * + 动态操作块（每个启用操作一块，2 列自适应；非喂食一点即记，喂食弹 FeedDialog）
 * + 最近记录摘要行。展示态口径见 lib/care.ts（视觉基线 mock-a-light）。
 * 冬眠卡：整卡灰化、操作块静音但仍可记账；「开始冬眠 / 确认出眠 / 补录冬眠」入口在卡片底部。
 * 记账/冬眠操作成功后抛 saved 让外层 refresh（数据驱动重算）。
 */
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyAction } from "../types";
import { actionTile, formatRecent, isFeeding, nowLocalDateTime, type TileView } from "../lib/care";
import { hibernationBanner } from "../lib/hibernation";
import { todayIso } from "../lib/dates";
import FeedDialog from "./FeedDialog.vue";
import HibernationDialog from "./HibernationDialog.vue";

const props = defineProps<{ colony: Colony }>();
const emit = defineEmits<{ edit: []; saved: [] }>();

const STATUS_TEXT: Record<Colony["status"], string> = {
  active: "● 活跃",
  hibernating: "❄ 冬眠中",
  ended: "◻ 已结束",
};

const hibernating = computed(() => props.colony.status === "hibernating");

/** 冬眠横幅数据（有开放段才显示；角标阈值 7 天 = spec 默认，纯展示，通知属票 06）。 */
const banner = computed(() =>
  hibernating.value && props.colony.hibernation !== null
    ? hibernationBanner(props.colony.hibernation, todayIso())
    : null,
);

const tiles = computed(() =>
  props.colony.actions.map((a) => ({ action: a, view: actionTile(a, hibernating.value) as TileView })),
);

const recentLine = computed(() => formatRecent(props.colony.recent));

const showFeed = ref(false);
const feedAction = ref<ColonyAction | null>(null);
const busyActionId = ref<number | null>(null);
const tileError = ref("");

const showHibernation = ref(false);
const hibernationMode = ref<"start" | "wake" | "past" | "edit">("start");

function openHibernation(mode: "start" | "wake" | "past" | "edit") {
  hibernationMode.value = mode;
  showHibernation.value = true;
}

function onHibernationSaved() {
  showHibernation.value = false;
  emit("saved");
}

function onTile(a: ColonyAction) {
  tileError.value = "";
  if (isFeeding(a)) {
    feedAction.value = a;
    showFeed.value = true;
    return;
  }
  void quickLog(a);
}

/** 非喂食一点即记：发生时间=现在（可后补，补录走记录页属票 04/05 范围）。 */
async function quickLog(a: ColonyAction) {
  busyActionId.value = a.action_id;
  try {
    await invoke("log_care", {
      input: {
        colony_id: props.colony.id,
        action_id: a.action_id,
        happened_at: nowLocalDateTime(),
        note: null,
        food_ids: [],
      },
    });
    emit("saved");
  } catch (e) {
    tileError.value = String(e);
  } finally {
    busyActionId.value = null;
  }
}

function onFeedSaved() {
  showFeed.value = false;
  feedAction.value = null;
  emit("saved");
}
</script>

<template>
  <article class="card" :class="{ hib: hibernating }" :data-colony-id="colony.id">
    <div class="chead">
      <div>
        <div class="cname">{{ colony.name }}</div>
        <div class="chips">
          <span v-if="colony.species" class="chip sp">{{ colony.species }}</span>
          <span class="chip st" :class="{ hib: colony.status === 'hibernating' }">
            {{ STATUS_TEXT[colony.status] }}
          </span>
        </div>
      </div>
      <div class="daysbox">
        <div class="n">{{ colony.days_raised }}</div>
        <div class="l">已饲养 / 天</div>
      </div>
    </div>
    <div class="start">开始饲养 {{ colony.start_date }}</div>

    <div v-if="banner" class="banner" data-testid="hib-banner">
      {{ banner.line }}
      <span v-if="banner.nearWake" class="chip wake">临近出眠</span>
      <button
        class="resched-btn"
        type="button"
        title="修改预计出眠日：未发的临近/出眠提醒按新日期重算"
        @click="openHibernation('edit')"
      >
        改期
      </button>
    </div>

    <div class="tiles">
      <button
        v-for="{ action: a, view } in tiles"
        :key="a.action_id"
        class="tile"
        :class="view.tone"
        :data-action-id="a.action_id"
        type="button"
        :disabled="busyActionId === a.action_id"
        @click="onTile(a)"
      >
        <span class="t-head">
          <span v-if="a.icon" class="t-ico">{{ a.icon }}</span>{{ a.name }}
          <span v-if="a.kind === 'log_only'" class="t-tag">仅登记</span>
        </span>
        <span class="pill">{{ view.text }}</span>
      </button>
    </div>
    <p v-if="tileError" class="tile-error">{{ tileError }}</p>

    <div v-if="recentLine" class="recent">{{ recentLine }}</div>

    <div class="card-actions">
      <button
        v-if="colony.status === 'active'"
        class="hib-btn"
        type="button"
        title="期间提醒静音、仍可记账"
        @click="openHibernation('start')"
      >
        ❄ 开始冬眠
      </button>
      <button
        v-if="hibernating"
        class="wake-btn"
        type="button"
        @click="openHibernation('wake')"
      >
        ☀ 确认出眠
      </button>
      <button
        v-if="colony.status !== 'ended'"
        class="past-btn"
        type="button"
        title="补录已闭合的过去冬眠段"
        @click="openHibernation('past')"
      >
        补录冬眠
      </button>
      <button class="edit-btn" type="button" @click="$emit('edit')">编辑</button>
    </div>

    <FeedDialog
      v-if="showFeed && feedAction !== null"
      :colony="colony"
      :action="feedAction"
      @close="showFeed = false"
      @saved="onFeedSaved"
    />
    <HibernationDialog
      v-if="showHibernation"
      :colony="colony"
      :mode="hibernationMode"
      @close="showHibernation = false"
      @saved="onHibernationSaved"
    />
  </article>
</template>

<style scoped>
.card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 16px;
  box-shadow: var(--shadow);
}

/* 冬眠整卡灰化（视觉照 mock-a-light 的 .card.hib：顶部向下渐隐的冷灰） */
.card.hib {
  background: linear-gradient(180deg, var(--hib-soft), var(--card) 55%);
}

.chead {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.cname {
  font-size: 17px;
  font-weight: 700;
}

.chips {
  margin-top: 3px;
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}

.chip {
  font-size: 12px;
  padding: 1px 9px;
  border-radius: 999px;
  border: 1px solid transparent;
}

.chip.sp {
  background: var(--accent-soft);
  color: var(--accent-deep);
}

.chip.st {
  background: var(--ok-soft);
  color: var(--ok);
}

.chip.st.hib {
  background: var(--hib-soft);
  color: var(--hib);
}

.daysbox {
  margin-left: auto;
  text-align: right;
}

.daysbox .n {
  font-size: 24px;
  font-weight: 800;
  line-height: 1.1;
}

.daysbox .l {
  font-size: 11px;
  color: var(--muted);
}

.start {
  margin-top: 6px;
  font-size: 12px;
  color: var(--muted);
}

/* ── 冬眠横幅（票 05，视觉照 mock-a-light 的 .banner/.chip.wake）── */
.banner {
  margin: 12px 0 4px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--hib-soft);
  color: var(--hib);
  font-size: 13px;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.chip.wake {
  background: var(--accent-soft);
  color: var(--accent-deep);
  font-weight: 600;
}

/* 横幅上的「改期」入口（票 09 停靠 D） */
.resched-btn {
  margin-left: auto;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--hib);
  font: inherit;
  font-size: 11px;
  padding: 1px 10px;
  border-radius: 999px;
  cursor: pointer;
  white-space: nowrap;
}

.resched-btn:hover {
  border-color: var(--hib);
  color: var(--text);
}

/* ── 操作块（视觉照 mock-a-light 的 .tiles/.tile/.t-tag/.pill）── */
.tiles {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  margin-top: 12px;
}

.tile {
  border: 1px solid var(--border);
  background: var(--tile);
  border-radius: 11px;
  padding: 10px 12px;
  cursor: pointer;
  text-align: left;
  font: inherit;
  color: var(--text);
  display: flex;
  flex-direction: column;
  gap: 6px;
  transition: 0.12s;
}

.tile:hover {
  border-color: var(--accent);
  transform: translateY(-1px);
}

.tile.ok:hover {
  box-shadow: 0 3px 10px rgba(180, 83, 9, 0.12);
}

.tile .t-head {
  display: flex;
  align-items: center;
  gap: 7px;
  font-weight: 600;
  font-size: 14px;
}

.tile .t-ico {
  font-size: 16px;
}

.t-tag {
  margin-left: auto;
  font-size: 10px;
  font-weight: 400;
  color: var(--muted);
  border: 1px solid var(--border);
  padding: 0 6px;
  border-radius: 999px;
  white-space: nowrap;
}

.pill {
  align-self: flex-start;
  font-size: 12px;
  padding: 1px 9px;
  border-radius: 999px;
}

.tile.reg .pill,
.tile.none .pill {
  background: var(--tile);
  color: var(--muted);
}

.tile.ok .pill {
  background: var(--ok-soft);
  color: var(--ok);
}

.tile.bad {
  border-color: var(--bad);
  background: var(--bad-soft);
}

.tile.bad:hover {
  box-shadow: 0 3px 10px rgba(209, 61, 61, 0.15);
}

.tile.bad .pill {
  background: #fff;
  color: var(--bad);
  font-weight: 600;
}

.tile.mute {
  opacity: 0.55;
  cursor: pointer;
}

.tile.mute .pill {
  background: var(--hib-soft);
  color: var(--hib);
}

.tile:disabled {
  opacity: 0.6;
  cursor: default;
  transform: none;
}

.tile-error {
  margin-top: 8px;
  font-size: 12px;
  color: var(--bad);
}

.recent {
  margin-top: 12px;
  font-size: 12px;
  color: var(--muted);
  border-top: 1px dashed var(--border);
  padding-top: 9px;
}

.card-actions {
  margin-top: 10px;
  display: flex;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 6px;
}

.edit-btn,
.hib-btn,
.wake-btn,
.past-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  padding: 2px 12px;
  border-radius: 8px;
  cursor: pointer;
}

.edit-btn:hover,
.hib-btn:hover,
.wake-btn:hover,
.past-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

/* 冬眠相关入口沿用冷灰系，与横幅呼应 */
.hib-btn,
.wake-btn {
  background: var(--hib-soft);
  color: var(--hib);
  border-color: var(--border);
}

.hib-btn:hover,
.wake-btn:hover {
  border-color: var(--hib);
  color: var(--text);
}
</style>

