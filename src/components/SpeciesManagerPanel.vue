<script setup lang="ts">
/**
 * 自建物种管理面板（species-profile 票 05，D19）：设置·物种 tab 的面板本体，
 * 行级组织沿 LocationManagerPanel 先例。只管自建物种——内置 12 种不进本清单
 * （不可删改、无停用开关，D4/D13）；清单来自 list_custom_species（后端只返自建）。
 *
 * - 改名：行内编辑，逐行「改名」即时落库（rename_custom_species，key 永不变）；
 *   引用窝的显示名快照级联刷新在后端，前端只需重新拉取清单 + 抛 changed 让
 *   外层刷新窝数据（卡片徽章随之更新）。
 * - 删除：被窝引用的行按钮置灰 + title 人话原因（前端禁用 + 后端引用守护双保险，
 *   D13「被引用禁删」）；未引用行直接删。
 * - 反馈（CLAUDE.md「操作反馈规范」）：改名/删除成败走全局唯一 toast；
 *   名字未改/空白是「无事可做」不弹不请求。清单异步首读走全局 LoadingHint，
 *   读取失败显示失败说明 + 重试；写后重拉时数据已在内存，不回加载态。
 *
 * 挂载方式：与地点/网页端/更新面板同为保活面板（v-show，弹窗打开即挂载）——
 * 行内未保存的改名草稿在页签往返间保留，弹窗关闭由外层整体销毁重读。
 */
import { onMounted, ref } from "vue";
import { deleteCustomSpecies, listCustomSpecies, renameCustomSpecies } from "../lib/ipc";
import type { CustomSpecies } from "../types";
import { showError, showSuccess } from "../lib/toast";
import LoadingHint from "./LoadingHint.vue";

/** 行状态：name 是行内草稿；origName 留存服务端现名（改名按钮启停与删除提示用） */
interface Row {
  key: string;
  name: string;
  origName: string;
  type: string;
  referenced: boolean;
}

const emit = defineEmits<{ changed: [] }>();

const loading = ref(true);
const loadError = ref("");
const rows = ref<Row[]>([]);
const busy = ref(false);

/** 被引用行的删除禁用原因（D13：被引用禁删，先在窝表单改选） */
const DELETE_GUARD_TITLE = "被窝引用，不能删除；请先在窝表单改选其他物种";

async function load(): Promise<void> {
  loading.value = true;
  loadError.value = "";
  try {
    rows.value = ((await listCustomSpecies()) ?? []).map((c: CustomSpecies) => ({
      key: c.key,
      name: c.name,
      origName: c.name,
      type: c.type,
      referenced: c.referenced,
    }));
  } catch (e) {
    loadError.value = String(e);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void load();
});

/** 有有效改动才可发（trim 后非空且 ≠ 现名） */
function dirty(row: Row): boolean {
  const t = row.name.trim();
  return t !== "" && t !== row.origName;
}

async function renameRow(row: Row): Promise<void> {
  if (!dirty(row) || busy.value) return;
  busy.value = true;
  try {
    await renameCustomSpecies({ key: row.key, newName: row.name.trim() });
    showSuccess(`已改名为「${row.name.trim()}」`);
    await load(); // 级联刷新引用窝快照在后端；前端重拉清单取最新名字/被引用态
    emit("changed"); // 外层刷新窝数据 → 卡片徽章即时跟上
  } catch (e) {
    showError("改名失败", String(e));
  } finally {
    busy.value = false;
  }
}

async function eraseRow(row: Row): Promise<void> {
  if (row.referenced || busy.value) return; // 置灰之外的双保险
  busy.value = true;
  try {
    await deleteCustomSpecies({ key: row.key });
    showSuccess(`已删除「${row.origName}」`);
    await load();
    emit("changed");
  } catch (e) {
    showError("删除失败", String(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="species-panel">
    <LoadingHint v-if="loading" />
    <template v-else>
      <p v-if="loadError" class="sp-load-error">
        读取失败：{{ loadError }}
        <button class="sp-reload-btn" type="button" @click="load">重试</button>
      </p>
      <template v-else>
        <p v-if="rows.length === 0" class="sp-empty">
          还没有自建物种。在窝表单的物种选择器里点「＋自建物种」即可创建；内置 12
          种在这里不显示（不可删改）。
        </p>
        <template v-else>
          <div v-for="row in rows" :key="row.key" class="sp-row">
            <input
              v-model="row.name"
              class="sp-name-input"
              type="text"
              :title="`类型：${row.type}；改名后引用窝的显示会自动跟着更新`"
              @keyup.enter="renameRow(row)"
            />
            <span class="sp-type">{{ row.type }}</span>
            <span v-if="row.referenced" class="sp-referenced-chip" title="有窝在用这个物种">被窝引用</span>
            <button
              class="sp-rename-btn"
              type="button"
              :disabled="!dirty(row) || busy"
              title="保存行内改的名字（key 不变，引用窝自动跟随）"
              @click="renameRow(row)"
            >
              改名
            </button>
            <button
              class="sp-erase-btn"
              type="button"
              :disabled="row.referenced || busy"
              :title="row.referenced ? DELETE_GUARD_TITLE : '删除这个自建物种'"
              @click="eraseRow(row)"
            >
              删除
            </button>
          </div>
        </template>
        <p class="sp-hint">
          这里只管理自建物种；内置 12 种不可删改。改名只改名字（编号不变），引用它的窝会自动显示新名字。
        </p>
      </template>
    </template>
  </div>
</template>

<style scoped>
.sp-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.sp-name-input {
  flex: 1;
  min-width: 90px;
  padding: 6px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.sp-type {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.sp-referenced-chip {
  font-size: 11px;
  color: var(--hib);
  background: var(--hib-soft);
  border-radius: 999px;
  padding: 1px 8px;
  white-space: nowrap;
}

.sp-row > button {
  border: 1px solid var(--border-strong);
  background: var(--card);
  border-radius: 8px;
  font: inherit;
  font-size: 12px;
  padding: 5px 10px;
  cursor: pointer;
  color: var(--muted);
  white-space: nowrap;
}

.sp-row > button:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.sp-row > button:disabled {
  opacity: 0.4;
  cursor: default;
}

.sp-empty {
  font-size: 13px;
  color: var(--muted);
}

.sp-load-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
  word-break: break-all;
}

.sp-reload-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  border-radius: 8px;
  font: inherit;
  font-size: 12px;
  padding: 4px 10px;
  cursor: pointer;
  color: var(--muted);
  margin-left: 6px;
}

.sp-hint {
  margin-top: 12px;
  font-size: 12px;
  color: var(--muted);
}
</style>
