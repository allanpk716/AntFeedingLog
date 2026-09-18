<script setup lang="ts">
/**
 * 窝卡片：名字、物种徽章、状态徽章、饲养天数大数字（Rust 算好）。
 * 操作块（喂食/换水/保湿/清理一按即记）属票 03，不在这里。
 */
import type { Colony } from "../types";

defineProps<{ colony: Colony }>();
defineEmits<{ edit: [] }>();

const STATUS_TEXT: Record<Colony["status"], string> = {
  active: "● 活跃",
  hibernating: "❄ 冬眠中",
  ended: "◻ 已结束",
};
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
    <div class="card-actions">
      <button class="edit-btn" type="button" @click="$emit('edit')">编辑</button>
    </div>
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
