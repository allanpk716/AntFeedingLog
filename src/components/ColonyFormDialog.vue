<script setup lang="ts">
/**
 * 新建/编辑窝弹窗：名字、物种、地点下拉、开始日期、状态。
 * 编辑模式额外提供「置为已结束」与「删除」（删除是否成功由 Rust 判定：仅无记录窝可删）。
 * 本组件自己发 IPC（create/update/archive/delete_colony），成功后抛 saved 让外层刷新。
 */
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyStatus, LocationItem } from "../types";
import { COLONY_STATUS_LABELS } from "../types";
import {
  emptyForm,
  formFromColony,
  formToInput,
  validateColonyForm,
} from "../lib/colonyForm";
import { todayIso } from "../lib/dates";

const props = defineProps<{
  editing: Colony | null;
  colonies: Colony[];
  locations: LocationItem[];
}>();
const emit = defineEmits<{ close: []; saved: [] }>();

const editingId = props.editing?.id ?? null;

// 新建默认挂第一个启用地点；编辑保留原地点
const defaultLocationId = props.editing?.location_id ?? props.locations.find((l) => l.enabled)?.id ?? null;

const form = ref(
  props.editing ? formFromColony(props.editing) : emptyForm(todayIso(), defaultLocationId),
);
const formError = ref("");
const busy = ref(false);

const statusOptions = Object.entries(COLONY_STATUS_LABELS).map(([value, label]) => ({
  value: value as ColonyStatus,
  label,
}));

// 编辑时原地点若已停用仍要能显示（规则 10 精神：原引用照常显示）
const locationOptions = computed<LocationItem[]>(() => {
  const enabled = props.locations.filter((l) => l.enabled);
  const current = props.editing?.location_id;
  if (current != null && !enabled.some((l) => l.id === current)) {
    const original = props.locations.find((l) => l.id === current);
    if (original) {
      return [...enabled, original];
    }
  }
  return enabled;
});

async function submit() {
  const error = validateColonyForm(form.value, props.colonies, editingId);
  if (error) {
    formError.value = error;
    return;
  }
  const input = formToInput(form.value);
  busy.value = true;
  formError.value = "";
  try {
    if (editingId === null) {
      await invoke("create_colony", { input });
    } else {
      await invoke("update_colony", { id: editingId, input });
    }
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function archive() {
  if (props.editing === null) return;
  busy.value = true;
  formError.value = "";
  try {
    await invoke("archive_colony", { id: props.editing.id });
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function remove() {
  if (props.editing === null) return;
  busy.value = true;
  formError.value = "";
  try {
    await invoke("delete_colony", { id: props.editing.id });
    emit("saved");
    emit("close");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="overlay">
    <div class="dialog">
      <h3>{{ editingId === null ? "新建窝" : "编辑窝" }}</h3>

      <div class="field-label">名字 *</div>
      <input
        v-model="form.name"
        class="name-input"
        type="text"
        placeholder="如：大头一号"
      />

      <div class="field-label">物种</div>
      <input
        v-model="form.species"
        class="species-input"
        type="text"
        placeholder="如：大头收获蚁"
      />

      <div class="field-label">地点</div>
      <select v-model="form.locationId" class="location-select">
        <option :value="null">未分组</option>
        <option v-for="loc in locationOptions" :key="loc.id" :value="loc.id">
          {{ loc.name }}{{ loc.enabled ? "" : "（已停用）" }}
        </option>
      </select>

      <div class="field-label">开始饲养日期 *</div>
      <input v-model="form.startDate" class="date-input" type="date" />

      <div class="field-label">状态</div>
      <select v-model="form.status" class="status-select">
        <option v-for="opt in statusOptions" :key="opt.value" :value="opt.value">
          {{ opt.label }}
        </option>
      </select>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <div class="dlg-btns">
        <button v-if="editingId !== null && form.status !== 'ended'" class="btn danger archive-btn" type="button" @click="archive">
          置为已结束
        </button>
        <button v-if="editingId !== null" class="btn danger delete-btn" type="button" @click="remove">
          删除
        </button>
        <span class="spacer"></span>
        <button class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary submit-btn" type="button" :disabled="busy" @click="submit">
          保存
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
  width: 400px;
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

.dialog input[type="text"],
.dialog input[type="date"],
.dialog select {
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
  align-items: center;
  gap: 8px;
  margin-top: 16px;
}

.spacer {
  flex: 1;
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

.btn.danger {
  color: var(--bad);
  border-color: var(--bad-soft);
}

.btn.danger:hover {
  border-color: var(--bad);
}

.btn:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
