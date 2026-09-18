<script setup lang="ts">
/**
 * 地点清单管理：改名、上下移调排序、新增、停用、删除。
 * 删除校验在 Rust：被窝引用的地点只能停用、不能物理删（评审附录规则 10）。
 * 保存 = 按当前行序逐行 save_location（sort = 行下标）。
 *
 * 关弹窗的只有两条路：整体「保存」成功、点取消。
 * 停用/删除这类行级操作只抛 changed 让外层静默刷新数据，弹窗保持打开，
 * 行内未保存的改名/排序也不受影响（行状态在本地维护，不因外层刷新重建）。
 */
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { LocationItem } from "../types";

interface Row {
  id: number | null;
  name: string;
  enabled: boolean;
}

const props = defineProps<{ locations: LocationItem[] }>();
const emit = defineEmits<{ close: []; saved: []; changed: [] }>();

function buildRows(locations: LocationItem[]): Row[] {
  return [...locations]
    .sort((a, b) => a.sort - b.sort || a.id - b.id)
    .map((l) => ({ id: l.id, name: l.name, enabled: l.enabled }));
}

const rows = ref<Row[]>(buildRows(props.locations));
const addName = ref("");
const error = ref("");
const busy = ref(false);

function move(index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= rows.value.length) return;
  const arr = rows.value;
  const tmp = arr[index]!;
  arr[index] = arr[target]!;
  arr[target] = tmp;
}

function addRow() {
  const name = addName.value.trim();
  if (!name) {
    error.value = "地点名字不能为空";
    return;
  }
  if (rows.value.some((r) => r.name.trim() === name)) {
    error.value = `地点名字「${name}」已存在`;
    return;
  }
  rows.value.push({ id: null, name, enabled: true });
  addName.value = "";
  error.value = "";
}

async function save() {
  const names = rows.value.map((r) => r.name.trim());
  if (names.some((n) => !n)) {
    error.value = "地点名字不能为空";
    return;
  }
  const duplicated = names.find((n, i) => names.indexOf(n) !== i);
  if (duplicated) {
    error.value = `地点名字「${duplicated}」已存在`;
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    for (let i = 0; i < rows.value.length; i++) {
      const row = rows.value[i]!;
      await invoke("save_location", { input: { id: row.id, name: row.name, sort: i } });
    }
    emit("saved");
    emit("close");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function deactivate(row: Row) {
  if (row.id === null) return;
  busy.value = true;
  error.value = "";
  try {
    await invoke("deactivate_location", { id: row.id });
    row.enabled = false;
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function erase(row: Row) {
  if (row.id === null) return;
  busy.value = true;
  error.value = "";
  try {
    await invoke("erase_location", { id: row.id });
    rows.value = rows.value.filter((r) => r !== row);
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="overlay">
    <div class="dialog loc-dialog">
      <h3>地点管理</h3>

      <div v-for="(row, index) in rows" :key="row.id ?? `new-${index}`" class="loc-row">
        <span class="loc-movers">
          <button class="move-up" type="button" :disabled="index === 0" @click="move(index, -1)">↑</button>
          <button class="move-down" type="button" :disabled="index === rows.length - 1" @click="move(index, 1)">↓</button>
        </span>
        <input v-model="row.name" class="loc-name-input" type="text" />
        <span v-if="!row.enabled" class="loc-disabled-chip">已停用</span>
        <button class="loc-deactivate-btn" type="button" :disabled="!row.enabled" @click="deactivate(row)">停用</button>
        <button class="loc-erase-btn" type="button" @click="erase(row)">删除</button>
      </div>

      <div class="loc-add">
        <input v-model="addName" class="loc-add-input" type="text" placeholder="新地点，如：阳台" />
        <button class="loc-add-btn" type="button" @click="addRow">＋ 添加</button>
      </div>

      <p v-if="error" class="loc-error">{{ error }}</p>

      <div class="dlg-btns">
        <span class="spacer"></span>
        <button class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary save-locations" type="button" :disabled="busy" @click="save">保存</button>
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

.loc-dialog {
  width: 480px;
  max-width: 92vw;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.loc-dialog h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

.loc-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.loc-movers {
  display: flex;
  gap: 2px;
}

.loc-movers button {
  border: 1px solid var(--border-strong);
  background: var(--tile);
  border-radius: 6px;
  cursor: pointer;
  font: inherit;
  padding: 4px 8px;
}

.loc-movers button:disabled {
  opacity: 0.35;
  cursor: default;
}

.loc-name-input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.loc-disabled-chip {
  font-size: 11px;
  color: var(--hib);
  background: var(--hib-soft);
  border-radius: 999px;
  padding: 1px 8px;
}

.loc-row > button {
  border: 1px solid var(--border-strong);
  background: var(--card);
  border-radius: 8px;
  font: inherit;
  font-size: 12px;
  padding: 5px 10px;
  cursor: pointer;
  color: var(--muted);
}

.loc-row > button:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.loc-row > button:disabled {
  opacity: 0.4;
  cursor: default;
}

.loc-add {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}

.loc-add input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.loc-add-btn {
  border: 1px dashed var(--border-strong);
  background: transparent;
  border-radius: 8px;
  font: inherit;
  padding: 6px 14px;
  cursor: pointer;
  color: var(--muted);
}

.loc-add-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.loc-error {
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

.btn:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
