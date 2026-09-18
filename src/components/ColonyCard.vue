<script setup lang="ts">
/**
 * 窝卡片：名字、物种徽章、状态徽章、饲养天数大数字（Rust 算好），
 * + 动态操作块（每个启用操作一块，2 列自适应；非喂食一点即记，喂食弹 FeedDialog）
 * + 最近记录摘要行。展示态口径见 lib/care.ts（视觉基线 mock-a-light）。
 * 记账成功后抛 saved 让外层 refresh（数据驱动重算）。
 */
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyAction } from "../types";
import { actionTile, formatRecent, isFeeding, nowLocalDateTime, type TileView } from "../lib/care";
import FeedDialog from "./FeedDialog.vue";

const props = defineProps<{ colony: Colony }>();
const emit = defineEmits<{ edit: []; saved: [] }>();

const STATUS_TEXT: Record<Colony["status"], string> = {
  active: "● 活跃",
  hibernating: "❄ 冬眠中",
  ended: "◻ 已结束",
};

const hibernating = computed(() => props.colony.status === "hibernating");

const tiles = computed(() =>
  props.colony.actions.map((a) => ({ action: a, view: actionTile(a, hibernating.value) as TileView })),
);

const recentLine = computed(() => formatRecent(props.colony.recent));

const showFeed = ref(false);
const feedAction = ref<ColonyAction | null>(null);
const busyActionId = ref<number | null>(null);
const tileError = ref("");

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
  <article class="card" :data-colony-id="colony.id">
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
      <button class="edit-btn" type="button" @click="$emit('edit')">编辑</button>
    </div>

    <FeedDialog
      v-if="showFeed && feedAction !== null"
      :colony="colony"
      :action="feedAction"
      @close="showFeed = false"
      @saved="onFeedSaved"
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
}

.edit-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  padding: 2px 12px;
  border-radius: 8px;
  cursor: pointer;
}

.edit-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}
</style>

