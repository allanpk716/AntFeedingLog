<script setup lang="ts">
/**
 * 喂食弹窗：食物多选 chips（仅启用食物）+ 时间（默认现在、可补录）+ 备注（可选）。
 * 本组件自持状态、自己发 IPC（list_foods / log_care），成功后抛 saved 让外层关窗刷新；
 * 停用食物不进新建入口（规则 10）。
 */
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyAction, FoodItem } from "../types";
import { nowLocalDateTime } from "../lib/care";

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

function toggleFood(id: number) {
  selectedIds.value = selectedIds.value.includes(id)
    ? selectedIds.value.filter((x) => x !== id)
    : [...selectedIds.value, id];
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
    <div class="dialog feed-dialog">
      <h3>记录喂食 · {{ colony.name }}</h3>

      <div class="field-label">食物（可多选）</div>
      <div class="foods">
        <button
          v-for="f in enabledFoods"
          :key="f.id"
          class="food"
          :class="{ selected: selectedIds.includes(f.id) }"
          type="button"
          @click="toggleFood(f.id)"
        >
          {{ f.name }}
        </button>
      </div>

      <div class="field-label">时间（默认现在，可补录）</div>
      <input v-model="time" class="time-input" type="datetime-local" />

      <div class="field-label">备注（可选）</div>
      <textarea v-model="note" class="note-input" placeholder="如：换了新一批面包虫"></textarea>

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

.dialog input[type="datetime-local"],
.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

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
