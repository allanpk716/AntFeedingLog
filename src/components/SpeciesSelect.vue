<script setup lang="ts">
/**
 * 物种选择器（species-profile 票 03）：替换窝表单的物种自由文本输入框。
 * - 类型分组：内置档案类型 distinct 动态成组（loader 保证，禁写死枚举）∪ 自建
 *   物种类型——自建同名类型并入既有组，新类型追加成组，分组恒动态；
 * - 冬眠需求筛选（三值）：只作用于内置档案——自建物种无档案数据，恒显示（筛
 *   选期间也能找到并选用自建物种）；
 * - 搜索：内置命中中文名/拉丁名/别名（大小写不敏感包含），自建命中名字/类型名；
 * - 悬停一句话摘要（温度/食物/冬眠，票 01 摘要助手）；自建无档案资料不显示；
 * - ＋自建物种内联新建（桌面专属：写命令不入网页端白名单）：名字必填（内联红
 *   字校验，不进轻提示），类型默认「自定义」可改填新类型名；与已有自建重名 →
 *   提示已存在并直接选用（后端 create 同语义归并，前端预检只为不发白发请求）；
 * - 物种可不填：「不指定」项清除选择（v-model 回空串），表单侧空串转 null。
 *
 * 数据与反馈规范：内置档案构建期在内存（打开面板即时渲染，不进加载态）；自建
 * 清单走 list_custom_species 异步拉取，首次打开显示全局 LoadingHint 占位，同
 * 一面板会话内缓存重开即时渲染（CLAUDE.md 加载态规范）。写操作反馈走全局
 * toast（操作反馈规范）：内联新建无天然反馈，成功绿色轻提示；IPC 失败红色带
 * 原因；重名选用同样轻提示说明「已存在」。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  allSpeciesProfiles,
  speciesSummary,
  type HibernationLevel,
} from "../lib/speciesProfiles";
import { createCustomSpecies, isTauri, listCustomSpecies } from "../lib/ipc";
import type { CustomSpecies } from "../types";
import { showError, showSuccess } from "../lib/toast";
import LoadingHint from "./LoadingHint.vue";

const props = defineProps<{ modelValue: string }>();
const emit = defineEmits<{ "update:modelValue": [key: string] }>();

const HIBERNATION_LEVELS: HibernationLevel[] = ["冬眠", "浅冬眠", "不冬眠"];
/** 自建物种默认类型（与后端 DEFAULT_TYPE 同值）。 */
const DEFAULT_CUSTOM_TYPE = "自定义";

const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const search = ref("");
const hibFilter = ref<"" | HibernationLevel>("");
/** null = 尚未拉到（首次加载中）；拉到后缓存在本组件会话内。 */
const customRows = ref<CustomSpecies[] | null>(null);
const customFailed = ref(false);
/** 清单一次会话只拉一次（挂载即拉，开面板兜底）；失败不自动重试（重开表单即重试）。 */
let customLoadStarted = false;

function ensureCustomLoaded(): void {
  if (customLoadStarted) return;
  customLoadStarted = true;
  void loadCustom();
}

// ── ＋自建物种内联新建 ──
const canCreateCustom = isTauri();
const creating = ref(false);
const newName = ref("");
const newType = ref(DEFAULT_CUSTOM_TYPE);
const newNameError = ref("");
const createBusy = ref(false);

interface SpeciesOption {
  key: string;
  label: string;
  /** 悬停摘要；自建无档案资料为空串（不渲染 title） */
  title: string;
  custom: boolean;
}

interface SpeciesGroup {
  type: string;
  items: SpeciesOption[];
}

/**
 * 面板选项：内置档案（冬眠筛选 + 三名搜索）→ 按类型首次出现序成组；自建清单
 * （不受冬眠筛选、按名字/类型搜索）→ 同名类型并入既有组、新类型追加在后。
 */
const groups = computed<SpeciesGroup[]>(() => {
  const q = search.value.trim().toLowerCase();
  const hib = hibFilter.value;
  const out: SpeciesGroup[] = [];
  const byType = new Map<string, SpeciesGroup>();
  const push = (type: string, item: SpeciesOption): void => {
    let group = byType.get(type);
    if (!group) {
      group = { type, items: [] };
      byType.set(type, group);
      out.push(group);
    }
    group.items.push(item);
  };
  for (const p of allSpeciesProfiles()) {
    if (hib !== "" && p.hibernation !== hib) continue;
    if (
      q !== "" &&
      !p.cnName.toLowerCase().includes(q) &&
      !p.latinName.toLowerCase().includes(q) &&
      !p.aliases.some((a) => a.toLowerCase().includes(q))
    ) {
      continue;
    }
    push(p.type, { key: p.key, label: p.cnName, title: speciesSummary(p), custom: false });
  }
  if (customRows.value !== null) {
    for (const c of customRows.value) {
      if (
        q !== "" &&
        !c.name.toLowerCase().includes(q) &&
        !c.type.toLowerCase().includes(q)
      ) {
        continue;
      }
      push(c.type, { key: c.key, label: c.name, title: "", custom: true });
    }
  }
  return out;
});

const hasAnyOption = computed(() => groups.value.some((g) => g.items.length > 0));

/** 触发器回显：内置=中文名、自建=名字、都解析不到原样显示 key（异常兜底）。 */
const currentLabel = computed<string>(() => {
  const key = props.modelValue.trim();
  if (key === "") return "";
  const p = allSpeciesProfiles().find((x) => x.key === key);
  if (p !== undefined) return p.cnName;
  const c = customRows.value?.find((x) => x.key === key);
  if (c !== undefined) return c.name;
  return key;
});

// ── 面板开合（点面板外即收，沿 ColonyCard 菜单先例）──

function openPanel(): void {
  open.value = true;
  search.value = "";
  hibFilter.value = "";
  creating.value = false;
  newNameError.value = "";
  ensureCustomLoaded();
}

function closePanel(): void {
  open.value = false;
}

function togglePanel(): void {
  open.value ? closePanel() : openPanel();
}

async function loadCustom(): Promise<void> {
  try {
    const rows = await listCustomSpecies();
    customRows.value = rows ?? [];
    customFailed.value = false;
  } catch {
    // 拉取失败：自建区降级为空 + 失败说明；内置照常可选，桌面仍可内联新建
    //（新建请求直接到后端，不受本清单影响）
    customRows.value = [];
    customFailed.value = true;
  }
}

function pick(key: string): void {
  emit("update:modelValue", key);
  closePanel();
}

function pickNone(): void {
  pick("");
}

function startCreating(): void {
  creating.value = true;
  newName.value = "";
  newType.value = DEFAULT_CUSTOM_TYPE;
  newNameError.value = "";
}

async function submitCreate(): Promise<void> {
  const name = newName.value.trim();
  if (name === "") {
    newNameError.value = "自建物种名字不能为空";
    return;
  }
  // 与已有自建重名：提示已存在并直接选用（后端 create 同名归并兜底）
  const existing = customRows.value?.find((r) => r.name === name);
  if (existing !== undefined) {
    showSuccess(`自建物种「${existing.name}」已存在，已直接选用`);
    pick(existing.key);
    return;
  }
  createBusy.value = true;
  try {
    const row = await createCustomSpecies({ name, speciesType: newType.value.trim() });
    customRows.value = [...(customRows.value ?? []), row];
    showSuccess(`已创建自建物种「${row.name}」并选用`);
    pick(row.key);
  } catch (e) {
    showError("自建物种创建失败", String(e));
  } finally {
    createBusy.value = false;
  }
}

function onDocClick(e: MouseEvent): void {
  if (!open.value) return;
  const root = rootRef.value;
  if (root !== null && e.target instanceof Node && root.contains(e.target)) return;
  closePanel();
}

onMounted(() => {
  document.addEventListener("click", onDocClick);
  // 挂载即拉自建清单：触发器对 custom-N 的回显（自建名）不依赖用户开面板
  ensureCustomLoaded();
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
});
</script>

<template>
  <div ref="rootRef" class="species-select">
    <button
      type="button"
      class="species-trigger"
      aria-haspopup="listbox"
      :aria-expanded="open"
      :aria-label="`物种：${currentLabel === '' ? '未指定' : currentLabel}，点开选择`"
      @click="togglePanel"
    >
      <span :class="currentLabel === '' ? 'placeholder' : 'picked-label'">
        {{ currentLabel === "" ? "未指定（可不选）" : currentLabel }}
      </span>
      <span class="caret" aria-hidden="true">▾</span>
    </button>

    <div v-if="open" class="species-panel">
      <div class="panel-controls">
        <input
          v-model="search"
          class="species-search"
          type="text"
          placeholder="搜索中文名/拉丁名/别名"
          aria-label="搜索物种"
        />
        <select v-model="hibFilter" class="hib-select" aria-label="按冬眠需求筛选">
          <option value="">全部冬眠需求</option>
          <option v-for="h in HIBERNATION_LEVELS" :key="h" :value="h">{{ h }}</option>
        </select>
      </div>

      <div class="option-list" role="listbox" aria-label="物种清单">
        <button
          type="button"
          class="option none-option"
          role="option"
          :class="{ picked: props.modelValue.trim() === '' }"
          title="不关联物种（可不填）"
          @click="pickNone"
        >
          不指定
        </button>

        <div v-for="g in groups" :key="g.type" class="species-group">
          <div class="group-label">{{ g.type }}</div>
          <button
            v-for="item in g.items"
            :key="item.key"
            type="button"
            class="option"
            role="option"
            :class="{ picked: props.modelValue.trim() === item.key }"
            :title="item.title === '' ? undefined : item.title"
            @click="pick(item.key)"
          >
            <span class="option-label">{{ item.label }}</span><span v-if="item.custom" class="custom-tag">自建</span>
          </button>
        </div>

        <!-- 自建清单异步拉取：全局唯一加载占位（内置档案在内存，不受影响即时可选） -->
        <LoadingHint v-if="customRows === null" />
        <p v-else-if="customFailed" class="custom-failed">自建物种清单加载失败，仅内置物种可选</p>
        <p v-else-if="!hasAnyOption" class="no-match">无匹配物种</p>
      </div>

      <template v-if="canCreateCustom">
        <button
          v-if="!creating"
          type="button"
          class="create-toggle"
          @click="startCreating"
        >
          ＋ 自建物种
        </button>
        <div v-else class="create-form">
          <div class="create-row">
            <input
              v-model="newName"
              class="create-name"
              type="text"
              placeholder="物种名字（必填）"
              aria-label="自建物种名字"
              @input="newNameError = ''"
            />
            <input
              v-model="newType"
              class="create-type"
              type="text"
              placeholder="类型"
              aria-label="自建物种类型"
            />
          </div>
          <p v-if="newNameError" class="create-error">{{ newNameError }}</p>
          <div class="create-btns">
            <button
              type="button"
              class="btn create-submit"
              :disabled="createBusy"
              @click="submitCreate"
            >
              创建并选用
            </button>
            <button
              type="button"
              class="btn create-cancel"
              :disabled="createBusy"
              @click="creating = false"
            >
              取消
            </button>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.species-select {
  position: relative;
}

.species-trigger {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
  cursor: pointer;
  text-align: left;
}

.species-trigger .placeholder {
  color: var(--muted);
}

.caret {
  color: var(--muted);
  flex: none;
}

.species-panel {
  position: absolute;
  left: 0;
  right: 0;
  top: calc(100% + 4px);
  z-index: 60;
  background: var(--card);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  box-shadow: 0 10px 30px rgba(60, 50, 30, 0.18);
  padding: 8px;
}

.panel-controls {
  display: flex;
  gap: 6px;
  margin-bottom: 6px;
}

/* 覆写宿主弹窗的 .dialog input[type="text"]/.dialog select 全宽规则：
   双类选择器压过它，搜索框占富余、筛选下拉收窄 */
.species-panel .panel-controls .species-search {
  flex: 1;
  min-width: 0;
  width: auto;
  padding: 5px 8px;
}

.species-panel .panel-controls .hib-select {
  flex: none;
  width: auto;
  padding: 5px 6px;
}

.option-list {
  max-height: 240px;
  overflow-y: auto;
}

.species-group {
  margin-top: 4px;
}

.group-label {
  font-size: 11px;
  color: var(--muted);
  padding: 3px 6px 1px;
}

.option {
  display: block;
  width: 100%;
  border: none;
  background: transparent;
  font: inherit;
  font-size: 13px;
  color: var(--text);
  padding: 5px 10px;
  border-radius: 7px;
  cursor: pointer;
  text-align: left;
}

.option:hover {
  background: var(--accent-soft);
  color: var(--accent-deep);
}

.option.picked {
  font-weight: 600;
}

.option.picked::after {
  content: " ✓";
  color: var(--accent-deep);
}

.option.none-option {
  color: var(--muted);
  border-bottom: 1px dashed var(--border);
  border-radius: 7px 7px 0 0;
}

.custom-tag {
  margin-left: 6px;
  font-size: 10px;
  color: var(--muted);
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 0 5px;
  vertical-align: 1px;
}

.no-match,
.custom-failed {
  margin: 6px;
  font-size: 12px;
  color: var(--muted);
}

.custom-failed {
  color: var(--bad);
}

.create-toggle {
  width: 100%;
  margin-top: 6px;
  border: 1px dashed var(--border-strong);
  background: transparent;
  font: inherit;
  font-size: 12px;
  color: var(--text);
  padding: 5px 8px;
  border-radius: 8px;
  cursor: pointer;
}

.create-toggle:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.create-form {
  margin-top: 6px;
  border-top: 1px dashed var(--border);
  padding-top: 6px;
}

.create-row {
  display: flex;
  gap: 6px;
}

/* 同 panel-controls：覆写宿主弹窗全宽规则 */
.create-form .create-row .create-name {
  flex: 1;
  min-width: 0;
  width: auto;
  padding: 5px 8px;
}

.create-form .create-row .create-type {
  flex: none;
  width: 88px;
  padding: 5px 8px;
}

.create-error {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--bad);
}

.create-btns {
  display: flex;
  gap: 6px;
  margin-top: 6px;
}

.create-btns .btn {
  padding: 4px 12px;
  font-size: 12px;
  border-radius: 8px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
}

.create-btns .create-submit {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.create-btns .create-submit:disabled,
.create-btns .create-cancel:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
