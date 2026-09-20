<script setup lang="ts">
/**
 * 新建/编辑窝弹窗：名字、物种、地点下拉、开始日期、状态。
 * 状态下拉只开放 活跃/已结束——冬眠只能走卡片「开始冬眠/确认出眠」流程（定点修 5）；
 * 编辑冬眠中的窝时该状态以禁用项显示、保存不改变它。
 * 编辑模式额外提供「置为已结束」与「删除」（删除是否成功由 Rust 判定：仅无记录窝可删）。
 * 编辑模式再有「周期提醒」小节（每窝周期票 04）：该窝每个启用操作一行可选周期，
 * 撤食不出现；其余行设/清走 set_colony_action_interval 逐行提交（现状不动）。
 * 巢穴保湿行（保湿方式票 02）变为「方式下拉 + 天数输入」，交互按规格状态矩阵
 * （选/切方式预填跟随、清回未设清空置灰、净零 B 仍 B），落库改走 update_colony
 * 整窗新通道（hydration_method + interval_changes，与基础字段同事务）。
 * 新建模式带「保湿方式」选择（D3 随建随落），保存走 create_colony 同一整窗通道。
 * 本组件自己发 IPC（create/update/archive/delete_colony、新建时 list_actions），
 * 成功后抛 saved 让外层刷新。
 */
import { computed, ref, watch } from "vue";
import {
  archiveColony,
  createColony,
  deleteColony,
  listActions,
  setColonyActionInterval,
  updateColony,
} from "../lib/ipc";
import type {
  Colony,
  ColonyInput,
  ColonyStatus,
  HydrationMethod,
  LocationItem,
} from "../types";
import { COLONY_STATUS_LABELS, HYDRATION_METHOD_LABELS } from "../types";
import {
  HYDRATION_ACTION_NAME,
  emptyForm,
  formFromColony,
  formToInput,
  hydrationAfterClear,
  hydrationAfterPick,
  intervalRowsFromColony,
  parseHydrationDays,
  planIntervalSaves,
  validateColonyForm,
  type HydrationSessionState,
  type IntervalRowInput,
  type IntervalSave,
} from "../lib/colonyForm";
import { todayIso } from "../lib/dates";
import DatePickerPop from "./DatePickerPop.vue";

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

// 状态下拉（终局评审定点修 5）：只开放 活跃/已结束（「置为已结束」是唯一解档通道）；
// 冬眠只能走卡片上的「开始冬眠 / 确认出眠」流程，不在编辑里手选。
// 编辑冬眠中的窝时追加一个禁用项，仅用于显示当前状态（改名保存不改状态）。
const selectableStatuses: { value: ColonyStatus; label: string }[] = [
  { value: "active", label: COLONY_STATUS_LABELS.active },
  { value: "ended", label: COLONY_STATUS_LABELS.ended },
];

const statusOptions = computed(() => {
  if (props.editing?.status !== "hibernating") return selectableStatuses;
  return [
    ...selectableStatuses,
    {
      value: "hibernating" as ColonyStatus,
      label: `${COLONY_STATUS_LABELS.hibernating}（用卡片上的「确认出眠」管理）`,
    },
  ];
});

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

// 「周期提醒」小节（每窝周期票 04，仅编辑模式）：新建时窝还不存在、无处设周期。
// 行 = 该窝每个启用操作（撤食不出现，tiles 本就只含启用操作）；已设行回显原始
// 每窝周期（tiles 契约：interval_from_colony=true 时 effective_interval_days 即原始值）。
const intervalRows = ref<IntervalRowInput[]>(
  props.editing ? intervalRowsFromColony(props.editing) : [],
);
// 回显基线（setup 时定格）：保存时只把相对基线有变化的行发给后端
const originalIntervalRows: IntervalRowInput[] = intervalRows.value.map((r) => ({ ...r }));

// ── 保湿方式（保湿方式票 02，规格状态矩阵四态九规则）─────────────────────
// 库内方式原值（setup 时定格）：F4 净零判定的会话基准——后端按「库内原值 vs
// 提交值」比较，会话内「选了又改回」对 B 态（未设+已有周期）净效果为零。
const storedHydrationMethod: HydrationMethod | null = props.editing?.hydration_method ?? null;
const hydrationMethod = ref<HydrationMethod | null>(storedHydrationMethod);
// 「手工改过」跟踪：prefill 非 null = 框内是本次会话预填的建议值且未被手工编辑
// （规则 3 的跟随判定依据；手工编辑即置 null，库内值/手工值切换方式时不动）。
const hydrationPrefill = ref<number | null>(null);
// 新建模式的天数框（编辑模式的框文本落在下方锚行对象上）
const hydrationRaw = ref("");

// 锚行：编辑模式按锚名在周期行里定位（与后端同名锚一致；改名/停用匹配不到为
// 已知边界——不出方式下拉、方式原样往返）。新建模式经 list_actions 异步取锚 id。
const hydrationRow: IntervalRowInput | null =
  props.editing === null
    ? null
    : (intervalRows.value.find((r) => r.actionName === HYDRATION_ACTION_NAME) ?? null);
const hydrationActionId: number | null = hydrationRow?.actionId ?? null;
const createHydrationActionId = ref<number | null>(null);

const hydrationMethodOptions = (Object.keys(HYDRATION_METHOD_LABELS) as HydrationMethod[]).map(
  (value) => ({ value, label: HYDRATION_METHOD_LABELS[value] }),
);

if (editingId === null) {
  // 新建：锚 id 取自操作字典（只认启用项，与编辑模式 tile 可见性同口径）；
  // 拿不到（改名/停用/查询失败）就降级为「只设方式标签、不落初始周期」。
  listActions()
    .then((items) => {
      createHydrationActionId.value =
        (items ?? []).find((a) => a.name === HYDRATION_ACTION_NAME && a.enabled)?.id ?? null;
    })
    .catch(() => {});
}

// 天数框文本统一读写口：编辑模式落在锚行对象上，新建落在独立框
function hydrationRawGet(): string {
  return hydrationRow !== null ? hydrationRow.raw : hydrationRaw.value;
}
function hydrationRawSet(v: string): void {
  if (hydrationRow !== null) hydrationRow.raw = v;
  else hydrationRaw.value = v;
}

// 显式清回未设且库内原值非未设：天数框清空置灰（规则 4，删除后果可见）
const hydrationLocked = computed(
  () => hydrationMethod.value === null && storedHydrationMethod !== null,
);

// 方式下拉 → 天数框：状态矩阵重算（规则 2/3/4/7 + F4/F5）。用 watch 而非
// @change，与 v-model 的赋值时序解耦，恒以新方式值参与判定。
watch(hydrationMethod, () => {
  const current: HydrationSessionState = {
    raw: hydrationRawGet(),
    prefill: hydrationPrefill.value,
  };
  const next =
    hydrationMethod.value === null
      ? hydrationAfterClear(current, storedHydrationMethod)
      : hydrationAfterPick(current, hydrationMethod.value);
  hydrationRawSet(next.raw);
  hydrationPrefill.value = next.prefill;
});

// 天数框手工编辑即脱离「会话预填」身份（规则 3「已手工改过」分支）
function onIntervalInput(row: IntervalRowInput): void {
  if (row.actionId === hydrationActionId) {
    hydrationPrefill.value = null;
  }
}

// 行内「清除」按钮：清天数留方式（规则 6）；保湿行连预填身份一起清
function clearIntervalRow(row: IntervalRowInput): void {
  row.raw = "";
  if (row.actionId === hydrationActionId) {
    hydrationPrefill.value = null;
  }
}

async function submit() {
  const error = validateColonyForm(form.value, props.colonies, editingId);
  if (error) {
    formError.value = error;
    return;
  }
  // 周期行先整组过校验：任一行非法就一行都不发（不落库，人话报错带上操作名）。
  // 编辑与其余行同组过 planIntervalSaves；新建的保湿框单独过同口径校验。
  let createHydrationDays: number | null = null;
  let plannedSaves: IntervalSave[] = [];
  if (editingId === null) {
    const hydrationDays = parseHydrationDays(hydrationRaw.value);
    if (typeof hydrationDays === "string") {
      formError.value = hydrationDays;
      return;
    }
    createHydrationDays = hydrationDays;
  } else {
    const plan = planIntervalSaves(intervalRows.value, originalIntervalRows);
    if (plan.error !== null) {
      formError.value = plan.error;
      return;
    }
    plannedSaves = plan.saves;
  }
  // 整窗入参：方式与周期增删随基础字段一次提交（spec F2/F6 同命令同事务）
  const input: ColonyInput = {
    ...formToInput(form.value),
    hydration_method: hydrationMethod.value,
    interval_changes: [],
  };
  busy.value = true;
  formError.value = "";
  try {
    if (editingId === null) {
      // 新建（D3 随建随落）：方式+初始周期与建窝同一条命令、同一事务
      if (createHydrationDays !== null && createHydrationActionId.value !== null) {
        input.interval_changes = [
          { action_id: createHydrationActionId.value, interval_days: createHydrationDays },
        ];
      }
      await createColony({ input });
    } else {
      // 保湿行的增删清改走整窗新通道。针对保湿行的 null（清除）只在方式保留
      // 时提交（C→D 只清天数）；清回未设/净零会话不发——前者由后端
      // clear_hydration 同事务删行，后者 F4 净零守卫本就丢弃、前端不白发。
      const hydrationSaves =
        hydrationActionId === null
          ? []
          : plannedSaves.filter((s) => s.actionId === hydrationActionId);
      input.interval_changes = hydrationSaves
        .filter((s) => !(hydrationMethod.value === null && s.intervalDays === null))
        .map((s) => ({ action_id: s.actionId, interval_days: s.intervalDays }));
      await updateColony({ id: editingId, input });
      // 其余操作行维持既有逐行通道（每窝周期票 04 现状不动）
      const others =
        hydrationActionId === null
          ? plannedSaves
          : plannedSaves.filter((s) => s.actionId !== hydrationActionId);
      for (const save of others) {
        await setColonyActionInterval({
          colonyId: editingId,
          actionId: save.actionId,
          intervalDays: save.intervalDays,
        });
      }
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
    await archiveColony({ id: props.editing.id });
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
    await deleteColony({ id: props.editing.id });
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
      <DatePickerPop v-model="form.startDate" placeholder="开始日期" />

      <div class="field-label">状态</div>
      <select v-model="form.status" class="status-select">
        <option
          v-for="opt in statusOptions"
          :key="opt.value"
          :value="opt.value"
          :disabled="editing?.status === 'hibernating' && opt.value === 'hibernating'"
        >
          {{ opt.label }}
        </option>
      </select>

      <!-- 保湿方式（保湿方式票 02，新建模式）：D3 随建随落，选方式预填默认可改 -->
      <template v-if="editingId === null">
        <div class="field-label">保湿方式</div>
        <div class="hydration-row">
          <select v-model="hydrationMethod" class="hydration-method-select" aria-label="保湿方式">
            <option :value="null">未设</option>
            <option v-for="opt in hydrationMethodOptions" :key="opt.value" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
          <template v-if="createHydrationActionId !== null && hydrationMethod !== null">
            <input
              v-model="hydrationRaw"
              class="interval-input"
              type="text"
              inputmode="numeric"
              placeholder="未设"
              aria-label="巢穴保湿的每窝周期（天）"
            />
            <span class="interval-unit">天</span>
          </template>
        </div>
        <p v-if="createHydrationActionId !== null" class="hydration-hint">
          选手动加水/水塔会自动填建议周期，可改；清空天数则不提醒
        </p>
      </template>

      <!-- 周期提醒（每窝周期票 04，仅编辑模式）：启用操作各一行，留空 = 未设。
           巢穴保湿行（保湿方式票 02）带方式下拉：选/切方式预填跟随、清回未设
           清空置灰（净零 B 仍 B），落库走 update_colony 整窗新通道。 -->
      <template v-if="editingId !== null">
        <div class="field-label">周期提醒</div>
        <p v-if="intervalRows.length === 0" class="interval-empty">该窝暂无启用的操作</p>
        <template v-for="row in intervalRows" :key="row.actionId">
          <div class="interval-row">
            <span class="interval-name">{{ row.actionName }}</span>
            <select
              v-if="row.actionId === hydrationActionId"
              v-model="hydrationMethod"
              class="hydration-method-select"
              :aria-label="`${row.actionName}的保湿方式`"
            >
              <option :value="null">未设</option>
              <option v-for="opt in hydrationMethodOptions" :key="opt.value" :value="opt.value">
                {{ opt.label }}
              </option>
            </select>
            <input
              v-model="row.raw"
              class="interval-input"
              type="text"
              inputmode="numeric"
              placeholder="未设"
              :aria-label="`${row.actionName}的每窝周期（天）`"
              :disabled="row.actionId === hydrationActionId && hydrationLocked"
              @input="onIntervalInput(row)"
            />
            <span class="interval-unit">天</span>
            <button
              class="btn interval-clear-btn"
              type="button"
              :aria-label="`清除${row.actionName}的每窝周期`"
              :disabled="row.actionId === hydrationActionId && hydrationLocked"
              @click="clearIntervalRow(row)"
            >
              清除
            </button>
          </div>
          <p v-if="row.actionId === hydrationActionId" class="hydration-hint">
            选手动加水/水塔会自动填建议周期，可改；清空天数则不提醒
          </p>
        </template>
      </template>

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
.dialog select {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

/* DatePickerPop 拉满行宽（对齐原 date 输入）：.dp 是 inline-block 收缩包围，
   button 的百分比宽拉不开父级——包壳改 block 才有效；触发器自身再占满 .dp */
.dialog :deep(.dp) {
  display: block;
  width: 100%;
}

.dialog :deep(.dp-trigger) {
  width: 100%;
}

/* 周期提醒行（每窝周期票 04）：名字占一行富余，天数输入窄条 + 清除小按钮。
   放在既有 .dialog input[type="text"] 通用规则之后，同特异性后者生效收窄宽度 */
.interval-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

.interval-name {
  flex: 1;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dialog input.interval-input {
  width: 76px;
  flex: none;
  padding: 5px 8px;
}

/* 保湿方式下拉（保湿方式票 02）：窄条嵌在行内，同特异性放在通用
   .dialog select 规则之后覆写宽度 */
.dialog select.hydration-method-select {
  width: 110px;
  flex: none;
  padding: 5px 8px;
}

.hydration-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

/* 选择器旁的一句说明（spec UI 节）：让「选方式→预填→可改」可懂 */
.hydration-hint {
  margin: 0 0 6px;
  font-size: 12px;
  color: var(--muted);
}

/* C 态清回未设：天数框同步清空置灰（规则 4，删除后果可见） */
.dialog input.interval-input:disabled,
.dialog select.hydration-method-select:disabled {
  opacity: 0.55;
  cursor: default;
}

.interval-unit {
  font-size: 12px;
  color: var(--muted);
  flex: none;
}

.btn.interval-clear-btn {
  flex: none;
  padding: 4px 10px;
  font-size: 12px;
}

.interval-empty {
  margin: 0;
  font-size: 12px;
  color: var(--muted);
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
