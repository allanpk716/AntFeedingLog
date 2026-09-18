<script setup lang="ts">
/**
 * 冬眠弹窗（票 05），四种模式：
 * - start「开始冬眠」：开始日默认今天；预计结束选完开始日自动预填开始日+120 天、可改
 *   （手动改过之后不再自动覆盖）；
 * - wake「确认出眠」：实际结束日期默认今天、可改（与预计不同没关系）；
 * - past「补录冬眠」：给已闭合的过去时间段补记录（两端都填，不碰窝当前状态）；
 * - edit「修改预计出眠」（票 09 停靠 D）：冬眠横幅「改期」入口，只改预计出眠日——
 *   未发的临近/出眠提醒由 Rust 侧按新日期自然重算（规则 2）。
 * 本组件自持状态、自己发 IPC，成功后抛 saved 让外层关窗刷新；
 * 约束（重叠/日期先后/已结束不可入眠等）权威校验在 Rust，错误串直接展示。
 */
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony } from "../types";
import { defaultExpectedEnd } from "../lib/hibernation";
import { todayIso } from "../lib/dates";

const props = defineProps<{
  colony: Colony;
  mode: "start" | "wake" | "past" | "edit";
}>();
const emit = defineEmits<{ close: []; saved: [] }>();

const title = computed(() => {
  if (props.mode === "start") return `开始冬眠 · ${props.colony.name}`;
  if (props.mode === "wake") return `确认出眠 · ${props.colony.name}`;
  if (props.mode === "edit") return `修改预计出眠 · ${props.colony.name}`;
  return `补录历史冬眠段 · ${props.colony.name}`;
});

const startDate = ref(todayIso());
const expectedEnd = ref(
  props.mode === "edit"
    ? props.colony.hibernation?.expected_end_date ?? todayIso()
    : defaultExpectedEnd(todayIso()),
);
const endTouched = ref(false);
const actualEnd = ref(todayIso());
const pastStart = ref("");
const pastEnd = ref("");
const formError = ref("");
const busy = ref(false);

watch(startDate, (v) => {
  if (!endTouched.value) {
    expectedEnd.value = defaultExpectedEnd(v);
  }
});

function submit() {
  formError.value = "";
  if (props.mode === "past" && (pastStart.value === "" || pastEnd.value === "")) {
    formError.value = "先选开始和结束日期";
    return;
  }
  if (props.mode === "edit" && expectedEnd.value === "") {
    formError.value = "先选新的预计出眠日";
    return;
  }
  busy.value = true;
  const done = () => {
    emit("saved");
  };
  const fail = (e: unknown) => {
    formError.value = String(e);
  };
  const finish = () => {
    busy.value = false;
  };
  if (props.mode === "start") {
    invoke("start_hibernation", {
      colonyId: props.colony.id,
      startDate: startDate.value,
      expectedEndDate: expectedEnd.value,
    })
      .then(done, fail)
      .finally(finish);
  } else if (props.mode === "wake") {
    invoke("confirm_wake", {
      colonyId: props.colony.id,
      actualEndDate: actualEnd.value,
    })
      .then(done, fail)
      .finally(finish);
  } else if (props.mode === "edit") {
    invoke("update_expected_end", {
      colonyId: props.colony.id,
      newExpectedEndDate: expectedEnd.value,
    })
      .then(done, fail)
      .finally(finish);
  } else {
    invoke("add_past_hibernation", {
      colonyId: props.colony.id,
      startDate: pastStart.value,
      endDate: pastEnd.value,
    })
      .then(done, fail)
      .finally(finish);
  }
}
</script>

<template>
  <div class="overlay" @click.self="$emit('close')">
    <div class="dialog hibernation-dialog">
      <h3>{{ title }}</h3>

      <template v-if="mode === 'start'">
        <div class="field-label">开始日期（默认今天）</div>
        <input v-model="startDate" class="start-input" type="date" />

        <div class="field-label">预计结束日期（默认开始日 + 120 天，可改）</div>
        <input
          v-model="expectedEnd"
          class="end-input"
          type="date"
          @input="endTouched = true"
        />
      </template>

      <template v-else-if="mode === 'edit'">
        <div class="field-label">新的预计出眠日（未发的提醒按新日期重算）</div>
        <input v-model="expectedEnd" class="end-input" type="date" />
      </template>

      <template v-else-if="mode === 'wake'">
        <div class="field-label">实际结束日期（默认今天，可与预计不同）</div>
        <input v-model="actualEnd" class="actual-input" type="date" />
      </template>

      <template v-else>
        <div class="field-label">历史段开始日期</div>
        <input v-model="pastStart" class="past-start-input" type="date" />
        <div class="field-label">历史段结束日期</div>
        <input v-model="pastEnd" class="past-end-input" type="date" />
      </template>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <div class="dlg-btns">
        <button class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary submit-btn" type="button" :disabled="busy" @click="submit">
          {{ mode === "start" ? "开始冬眠" : mode === "wake" ? "确认出眠" : mode === "edit" ? "保存新日期" : "补录" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 与 FeedDialog 同基座的日期弹窗（视觉基线 mocks/mock-a-light.html） */
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
  width: 360px;
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

.dialog input[type="date"] {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
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
