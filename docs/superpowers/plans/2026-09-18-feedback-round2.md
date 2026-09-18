# AntFeedingLog 反馈第二轮实施计划（F1–F4）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地用户实测反馈的五项决议：快捷打卡面板、字典预置项禁删、双层喂食周期、通知双通道（Windows toast + Pushover）与单总开关。

**Architecture:** 沿用既有分层——数据与领域逻辑全在 Rust（`&Connection` 纯函数 + cargo test），Tauri command 薄包装，前端只做展示态拼装（vitest）。schema 经 `PRAGMA user_version` 逐级迁移：v4（is_preset）→ v5（台账推送列）→ v6（食物周期 + 台账食物维度）。Pushover 凭证只读环境变量，不进库不进备份；HTTP 经注入式传输层，单测不真发。

**Tech Stack:** Tauri 2 / Vue 3 / rusqlite / vitest / 新增依赖 `ureq = "2"`（唯一新依赖）。

**Spec:** `docs/superpowers/specs/20260918-ant-feeding-log-feedback2-consensus.md`（决议总表 Q1–Q10 + 排除项 + 边界，实施前必读）

**执行顺序：** F1 → F2 → F4 → F3（串行；F3 与 F1 都改 `ColonyCard.vue`、F2/F3 都改食物字典、F3 的台账改造建立在 F4 的 v5 之上）。对应票文件 `.scratch/ant-feeding-log/issues/10-13`。

## Global Constraints

- 工作在 worktree 分支（using-git-worktrees），不动 main、不 push；每步 TDD（先失败测试再实现），每任务一提交。
- Rust 测试只测外部行为，`now`/`today` 一律注入，不碰时钟、不碰网络（Pushover 的 HTTP 经参数注入）。
- 错误文案面向用户（中文、含「停用」等出路提示），与 dict.rs 现有风格一致。
- 迁移只升不降、逐版本 match 分支、版本号钉死；`SCHEMA_VERSION` 常量同步 +1。
- 前端类型与 Rust DTO 严格 snake_case 对齐（`src/types.ts`）；改 DTO 必同步改 types 与相关测试夹具。
- 测试命令：Rust `cargo test --manifest-path src-tauri/Cargo.toml`；前端 `pnpm test`；类型检查 `pnpm build`（vue-tsc）。
- 决议排除项不得顺手实现：首页不加删除入口、不做撤销条、Pushover 不做设置输入框、统计页不加食物维度、通知不做分类子开关。

---

### Task 1（票 F1）：快捷打卡面板

**Files:**
- Create: `src/components/QuickLogDialog.vue`
- Create: `src/components/QuickLogDialog.test.ts`
- Modify: `src/components/ColonyCard.vue`（onTile 分流、删 quickLog/busyActionId/tileError）
- Modify: `src/App.test.ts`（两处一点即记测试改为面板流程）

**Interfaces:**
- Consumes: 既有 `log_care` command（入参 `CareLogInput`，后端零改动）；`nowLocalDateTime()`（src/lib/care.ts）。
- Produces: 组件 `QuickLogDialog`，props `{ colony: Colony; action: ColonyAction }`，emits `close / saved`；根元素类名 `.quick-dialog`，内部 `.time-input / .note-input / .record-btn / .cancel-btn / .form-error`（与 FeedDialog 同命名约定，测试按类名找）。

- [ ] **Step 1.1: 写失败测试（组件级）**

新建 `src/components/QuickLogDialog.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import QuickLogDialog from "./QuickLogDialog.vue";
import type { Colony, ColonyAction } from "../types";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const colony: Colony = {
  id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20",
  status: "active", days_raised: 241, actions: [], recent: [], hibernation: null,
};
const action: ColonyAction = {
  action_id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false,
  suggested_interval_days: null, days_since_last: 2, overdue: false, foods: [],
};

function mountDlg() {
  return mount(QuickLogDialog, { props: { colony, action } });
}

beforeEach(() => { invokeMock.mockReset(); });

describe("QuickLogDialog", () => {
  it("取消：直接关闭，不触发 log_care", async () => {
    const w = mountDlg();
    await w.find(".cancel-btn").trigger("click");
    expect(invokeMock).not.toHaveBeenCalled();
    expect(w.emitted("close")).toHaveLength(1);
  });

  it("提交：log_care 带默认现在时间、备注 null、空食物，成功后抛 saved", async () => {
    invokeMock.mockResolvedValue(3);
    const w = mountDlg();
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(call).toBeDefined();
    const input = call![1] as { input: Record<string, unknown> };
    expect(input.input.colony_id).toBe(1);
    expect(input.input.action_id).toBe(2);
    expect(input.input.happened_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
    expect(input.input.note).toBeNull();
    expect(input.input.food_ids).toEqual([]);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("改补录时间 + 备注 trim 后提交", async () => {
    invokeMock.mockResolvedValue(4);
    const w = mountDlg();
    await w.find(".time-input").setValue("2026-09-17T21:30");
    await w.find(".note-input").setValue("  顺手清了垃圾区  ");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const input = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care")![1] as { input: Record<string, unknown> };
    expect(input.input.happened_at).toBe("2026-09-17T21:30");
    expect(input.input.note).toBe("顺手清了垃圾区");
  });

  it("提交失败：错误展示在弹窗内、弹窗不关（未来时间被后端拒）", async () => {
    invokeMock.mockRejectedValue("发生时间不能晚于当前时间（2026-09-19T09:00 在未来）");
    const w = mountDlg();
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").text()).toContain("未来");
    expect(w.find(".quick-dialog").exists()).toBe(true);
    expect(w.emitted("saved")).toBeUndefined();
  });
});
```

注：`ColonyAction.foods` 字段 Task 4（F3）才加——本任务夹具先写 `foods: []` 会类型报错。**处理**：夹具暂不含 `foods`（types.ts 尚无该字段，Task 4 加字段时再补夹具）。上面代码里的 `foods: []` 删掉。

- [ ] **Step 1.2: 跑测试确认失败**

Run: `pnpm vitest run src/components/QuickLogDialog.test.ts`
Expected: FAIL（找不到组件模块）

- [ ] **Step 1.3: 实现 QuickLogDialog.vue**

结构照 FeedDialog.vue 抄（overlay/dialog/field-label/input/textarea/dlg-btns/btn 及对应 scoped 样式整段复制），去掉食物区：

```vue
<script setup lang="ts">
/**
 * 快捷打卡面板（反馈第二轮 F1，Q1=C）：非喂食操作点击后先弹此面板——
 * 日期默认今天、可补录过去、可填备注，点「记录」才落库；取消不产生任何记录。
 * 喂食不弹这里（FeedDialog 自带食物多选）。后端 log_care 已拒未来时间。
 */
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Colony, ColonyAction } from "../types";
import { nowLocalDateTime } from "../lib/care";

const props = defineProps<{ colony: Colony; action: ColonyAction }>();
const emit = defineEmits<{ close: []; saved: [] }>();

const time = ref(nowLocalDateTime());
const note = ref("");
const formError = ref("");
const busy = ref(false);

async function submit() {
  busy.value = true;
  formError.value = "";
  try {
    await invoke("log_care", {
      input: {
        colony_id: props.colony.id,
        action_id: props.action.action_id,
        happened_at: time.value,
        note: note.value.trim() === "" ? null : note.value.trim(),
        food_ids: [],
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
    <div class="dialog quick-dialog">
      <h3>记录{{ action.name }} · {{ colony.name }}</h3>

      <div class="field-label">时间（默认现在，可补录）</div>
      <input v-model="time" class="time-input" type="datetime-local" />

      <div class="field-label">备注（可选）</div>
      <textarea v-model="note" class="note-input" placeholder="如：顺手检查了垃圾区"></textarea>

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
```

（scoped 样式从 FeedDialog.vue 104-214 行整段复制。）

- [ ] **Step 1.4: 跑组件测试确认通过**

Run: `pnpm vitest run src/components/QuickLogDialog.test.ts`
Expected: 4 PASS

- [ ] **Step 1.5: 改 ColonyCard.vue 接线**

`<script setup>` 改动：
- import 行加 `import QuickLogDialog from "./QuickLogDialog.vue";`
- 删除 `busyActionId`、`tileError` 两个 ref 与整个 `quickLog` 函数（71-90 行）。
- `onTile` 非喂食分支改为开面板：

```ts
const showQuick = ref(false);
const quickAction = ref<ColonyAction | null>(null);

function onTile(a: ColonyAction) {
  if (isFeeding(a)) {
    feedAction.value = a;
    showFeed.value = true;
    return;
  }
  quickAction.value = a;
  showQuick.value = true;
}

function onQuickSaved() {
  showQuick.value = false;
  quickAction.value = null;
  emit("saved");
}
```

`<template>` 改动：
- tile 按钮删 `:disabled="busyActionId === a.action_id"`。
- 删 `<p v-if="tileError" class="tile-error">` 行与 `.tile-error` 样式块。
- FeedDialog 之后加：

```html
<QuickLogDialog
  v-if="showQuick && quickAction !== null"
  :colony="colony"
  :action="quickAction"
  @close="showQuick = false"
  @saved="onQuickSaved"
/>
```

- [ ] **Step 1.6: 改 App.test.ts 两处一点即记测试**

①「非喂食一点即记…」（约 902-928 行）改为：

```ts
it("非喂食弹打卡面板：默认今天可补录，点「记录」才落库并刷新", async () => {
  colony1With([waterReg]);
  const wrapper = await mountApp();

  await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
  const dialog = wrapper.find(".quick-dialog");
  expect(dialog.exists()).toBe(true);
  expect(dialog.find("h3").text()).toBe("记录活动区换水 · 大头一号");

  await dialog.find(".time-input").setValue("2026-09-17T21:00");
  colony1With([{ ...waterReg, days_since_last: 0 }]);
  invokeMock.mockClear();
  await dialog.find(".record-btn").trigger("click");
  await flushPromises();

  const logCall = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
  expect(logCall).toBeDefined();
  const input = logCall![1] as { input: { colony_id: number; action_id: number; happened_at: string } };
  expect(input.input.colony_id).toBe(1);
  expect(input.input.action_id).toBe(2);
  expect(input.input.happened_at).toBe("2026-09-17T21:00");
  expect(wrapper.find(".quick-dialog").exists()).toBe(false);
  expect(wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"] .pill').text()).toBe("今天 · 已记录");
});

it("打卡面板点「取消」不记账", async () => {
  colony1With([waterReg]);
  const wrapper = await mountApp();
  await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
  invokeMock.mockClear();
  await wrapper.find(".quick-dialog .cancel-btn").trigger("click");
  await flushPromises();
  expect(wrapper.find(".quick-dialog").exists()).toBe(false);
  expect(invokeMock.mock.calls.some(([cmd]) => cmd === "log_care")).toBe(false);
});
```

②「一键记账失败：卡片上展示原因」（约 1014 行起）改为面板内报错：

```ts
it("打卡面板提交失败：原因展示在面板内、面板不关", async () => {
  colony1With([waterReg]);
  const wrapper = await mountApp();
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "log_care": throw "操作「活动区换水」已停用，不能新记";
      case "list_colonies": return currentColonies;
      case "list_locations": return locations;
      default: return null;
    }
  });
  await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
  await wrapper.find(".quick-dialog .record-btn").trigger("click");
  await flushPromises();
  expect(wrapper.find(".quick-dialog .form-error").text()).toContain("停用");
  expect(wrapper.find(".quick-dialog").exists()).toBe(true);
});
```

（原测试结尾对 `.tile-error` 的断言一并删除。）

- [ ] **Step 1.7: 全量验证 + 提交**

Run: `pnpm test`（全绿）→ `pnpm build`（vue-tsc 过）→ `cargo test --manifest-path src-tauri/Cargo.toml`（应零改动全绿）。

```bash
git add src/components/QuickLogDialog.vue src/components/QuickLogDialog.test.ts src/components/ColonyCard.vue src/App.test.ts
git commit -m "feat(quicklog): 非喂食打卡改弹确认面板——日期可补录+备注，取消零落库（票 F1）"
```

---

### Task 2（票 F2）：字典预置项禁删

**Files:**
- Modify: `src-tauri/src/db.rs`（SCHEMA_VERSION 4 + migrate_v3_to_v4）
- Modify: `src-tauri/src/dict.rs`（DTO/SQL/erase 守护）
- Modify: `src-tauri/src/care.rs`（Food DTO/FOOD_SQL/erase 用不上——erase_food 在 dict.rs，care 只改 Food 与 FOOD_SQL）
- Modify: `src/types.ts`、`src/lib/dict.ts`、`src/components/SettingsDialog.vue`
- Test: `src-tauri/src/db.rs`、`src-tauri/src/dict.rs` 内嵌 tests；`src/lib/dict.test.ts`

**Interfaces:**
- Produces（Rust）: `CareAction.is_preset: bool`、`care::Food.is_preset: bool`；`erase_action`/`erase_food` 对预置项返回 `Err("预置操作不能删除；可改为停用")` / `Err("预置食物不能删除；可改为停用")`。
- Produces（前端）: `CareActionItem.is_preset`、`FoodItem.is_preset`、`ActionRow.isPreset`、`FoodRow.isPreset`。

- [ ] **Step 2.1: 写失败测试（db 迁移）**

db.rs tests 内新增（并把 `user_version_is_schema_version` 的 `assert_eq!(SCHEMA_VERSION, 3)` 改为 4）：

```rust
#[test]
fn v4_flags_preset_actions_and_foods() {
    let (conn, _dir) = fresh_conn();
    let action_names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM care_action WHERE is_preset = 1 ORDER BY sort").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(action_names, vec!["喂食", "活动区换水", "巢穴保湿", "垃圾清理"]);
    let food_names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM food WHERE is_preset = 1 ORDER BY sort").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(food_names, vec!["种子", "干虾仁", "面包虫"]);
}

#[test]
fn v3_db_with_custom_rows_upgrades_to_v4_and_backfills() {
    // 真实 v3 库 + 用户自建行 + 一个改过名的预置（回填盲区，见共识文档边界）
    let conn = Connection::open_in_memory().expect("内存库打开失败");
    conn.execute_batch(V1_SCHEMA_SQL).unwrap();
    seed_v1_presets(&conn).unwrap();
    migrate_v1_to_v2(&conn).unwrap();
    migrate_v2_to_v3(&conn).unwrap();
    conn.execute("UPDATE care_action SET name = '换水' WHERE name = '活动区换水'", []).unwrap();
    conn.execute("INSERT INTO care_action (name, kind, enabled, sort) VALUES ('降温', 'log_only', 1, 5)", []).unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();

    migrate(&conn).unwrap();
    assert_eq!(scalar_i64(&conn, "PRAGMA user_version"), SCHEMA_VERSION);
    let flagged: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM care_action WHERE is_preset = 1 ORDER BY sort").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    // 已改名的预置匹配不到（已知边界，接受）；自建项不误标
    assert_eq!(flagged, vec!["喂食", "巢穴保湿", "垃圾清理"]);
}
```

- [ ] **Step 2.2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml db::`
Expected: FAIL（v4 测试红、版本断言红）

- [ ] **Step 2.3: 实现 v4 迁移**

db.rs：`SCHEMA_VERSION` 改 4（注释补一行 v4 说明）；`migrate` match 加 `3 => migrate_v3_to_v4(conn)?,`；新增：

```rust
/// v3 → v4：字典预置项保护位（反馈第二轮 F2，Q5）。care_action/food 各加 is_preset，
/// 按名字回填预置行（喂食/活动区换水/巢穴保湿/垃圾清理、种子/干虾仁/面包虫）。
/// 已改名且从未被引用的预置回填不到——已知边界（单人工具可接受，见共识文档）。
/// 新库走同一条路：v0→v1 seed 不带该列，v4 统一回填。
fn migrate_v3_to_v4(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE care_action
            ADD COLUMN is_preset INTEGER NOT NULL DEFAULT 0 CHECK (is_preset IN (0, 1));
        UPDATE care_action SET is_preset = 1
            WHERE name IN ('喂食', '活动区换水', '巢穴保湿', '垃圾清理');

        ALTER TABLE food
            ADD COLUMN is_preset INTEGER NOT NULL DEFAULT 0 CHECK (is_preset IN (0, 1));
        UPDATE food SET is_preset = 1 WHERE name IN ('种子', '干虾仁', '面包虫');
        "#,
    )?;
    tx.pragma_update(None, "user_version", 4)?;
    tx.commit()
}
```

- [ ] **Step 2.4: db 测试转绿**

Run: `cargo test --manifest-path src-tauri/Cargo.toml db::`
Expected: PASS

- [ ] **Step 2.5: 写失败测试（dict/care 守护与 DTO）**

dict.rs tests 新增：

```rust
#[test]
fn erase_action_rejects_all_four_presets_even_unreferenced() {
    let conn = mem_conn();
    for name in ["喂食", "活动区换水", "巢穴保湿", "垃圾清理"] {
        let err = erase_action(&conn, action_id(&conn, name)).unwrap_err();
        assert!(err.contains("预置"), "{name} 应拒删，实际：{err}");
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM care_action"), 4);
    // 改名/停用不受影响
    let water = action_id(&conn, "活动区换水");
    save_action(&conn, &ActionInput { id: Some(water), name: "换水".into(), kind: "log_only".into(), is_feeding: false, suggested_interval_days: None, sort: 2 }).unwrap();
    set_action_enabled(&conn, water, false).unwrap();
}

#[test]
fn erase_food_rejects_three_presets() {
    let conn = mem_conn();
    for name in ["种子", "干虾仁", "面包虫"] {
        let err = erase_food(&conn, food_id(&conn, name)).unwrap_err();
        assert!(err.contains("预置"), "{name} 应拒删，实际：{err}");
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM food"), 3);
}

#[test]
fn list_actions_and_foods_expose_is_preset() {
    let conn = mem_conn();
    let created = save_action(&conn, &ActionInput { id: None, name: "降温".into(), kind: "log_only".into(), is_feeding: false, suggested_interval_days: None, sort: 5 }).unwrap();
    let actions = list_actions(&conn).unwrap();
    assert!(by_name(&actions, "喂食").is_preset);
    assert!(!actions.iter().find(|a| a.id == created.id).unwrap().is_preset);
    let foods = crate::care::list_foods(&conn).unwrap();
    assert!(foods.iter().find(|f| f.name == "种子").unwrap().is_preset);
    let custom = save_food(&conn, &FoodInput { id: None, name: "糖水".into(), sort: 9 }).unwrap();
    assert!(!custom.is_preset);
}
```

dict.test.ts（前端）新增：`buildActionRows`/`buildFoodRows` 映射 `isPreset`；夹具补 `is_preset` 字段（现有夹具全部加 `is_preset: false`，预置行按需 true）。

```ts
it("预置位映射到行：isPreset 跟随 is_preset", () => {
  const rows = buildActionRows([
    { ...baseAction, id: 1, name: "喂食", is_preset: true },
    { ...baseAction, id: 2, name: "降温", is_preset: false },
  ]);
  expect(rows[0]!.isPreset).toBe(true);
  expect(rows[1]!.isPreset).toBe(false);
});
```

- [ ] **Step 2.6: 确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dict::` 与 `pnpm vitest run src/lib/dict.test.ts`
Expected: FAIL

- [ ] **Step 2.7: 实现守护与 DTO**

dict.rs：
- `CareAction` 加 `pub is_preset: bool,`
- `ACTION_SQL` 列序改为 `..., a.enabled, a.sort, a.is_preset, (EXISTS(...))`；`row_to_action` 相应 `is_preset: row.get::<_, i64>(8)? != 0, referenced: row.get::<_, i64>(9)? != 0`。
- `erase_action` 最前面加：

```rust
let preset: i64 = conn
    .query_row("SELECT is_preset FROM care_action WHERE id = ?1", params![id], |r| r.get(0))
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => "操作不存在".to_string(),
        other => db_err(other),
    })?;
if preset == 1 {
    return Err("预置操作不能删除；可改为停用".into());
}
```

- `erase_food` 同款（查 `food.is_preset`，文案"预置食物不能删除；可改为停用"）。

care.rs：
- `Food` 加 `pub is_preset: bool,`
- `FOOD_SQL` 改为 `SELECT f.id, f.name, f.enabled, f.sort, f.is_preset, EXISTS(...)`；`row_to_food` 加 `is_preset: row.get::<_, i64>(4)? != 0, referenced: row.get::<_, i64>(5)? != 0`。

前端：
- `types.ts`：`CareActionItem`、`FoodItem` 各加 `is_preset: boolean`（doc 注释：预置项禁删可停用）。
- `dict.ts`：`ActionRow`/`FoodRow` 加 `isPreset: boolean`；`buildActionRows`/`buildFoodRows` 映射。
- `SettingsDialog.vue`：`eraseTitle` 改为吃整行：

```ts
function eraseTitle(row: { referenced: boolean; isPreset: boolean }): string {
  if (row.isPreset) return "预置项不能删除；可改为停用";
  return row.referenced ? "被历史记录引用，只能停用，不能删除" : "";
}
```

操作/食物两处删除按钮：`:disabled="row.referenced || row.isPreset" :title="eraseTitle(row)"`。
- `App.test.ts` 顶部 `actions`/`foods` 夹具每行补 `is_preset: true/false`（预置行 true；`蚕蛹` false）。

- [ ] **Step 2.8: 全量验证 + 提交**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` + `pnpm test` + `pnpm build`。
注：care.rs/dict.rs 既有测试的 `Food`/`CareAction` 构造处若因新字段编译红，补 `is_preset: false`（或预置语义 true）即可——机械修改。

```bash
git add -A src-tauri/src src/types.ts src/lib/dict.ts src/lib/dict.test.ts src/components/SettingsDialog.vue src/App.test.ts
git commit -m "feat(dict): 预置四操作三食物禁删（is_preset，v4 迁移按名回填）；改名停用照常（票 F2）"
```

---

### Task 3（票 F4）：通知双通道 + Pushover + 单总开关

**Files:**
- Modify: `src-tauri/Cargo.toml`（加 `ureq = "2"`）
- Create: `src-tauri/src/pushover.rs`
- Modify: `src-tauri/src/db.rs`（v5：台账 push 列）
- Modify: `src-tauri/src/reminder.rs`（run_check 拆 IO、重试、测试通知返回结构、删分类过滤）
- Modify: `src-tauri/src/lib.rs`（mod pushover、pushover_status 命令、send_test_notification 返回值）
- Modify: `src/lib/notifySettings.ts`、`src/components/SettingsDialog.vue`、`src/types.ts`、相关测试

**Interfaces:**
- Produces（pushover.rs）:
  - `pub const API_URL / ENV_USER("PUSHOVER_USER") / ENV_TOKEN("PUSHOVER_TOKEN") / TIMEOUT_SECS(10)`
  - `pub struct PushoverConfig { user, token }` + `from_env() -> Option<Self>`
  - `pub fn send_with(cfg, title, message, post: &dyn Fn(&str, &Form) -> Result<(), String>) -> Result<(), String>`（`pub type Form = Vec<(String, String)>`）
  - `pub fn send(cfg, title, message) -> Result<(), String>`（ureq 真发）
  - `pub struct PushoverStatus { user_found, token_found }` + `status_from_env()`
- Produces（reminder.rs）:
  - `pub struct PushJob { pub ledger_id: i64, pub title: String, pub body: String }`
  - `pub struct CheckOutcome { pub toasts: Vec<Reminder>, pub push_jobs: Vec<PushJob> }`
  - `run_check(conn, today, now) -> Result<CheckOutcome, String>`（IO 全部移出）
  - `pub fn settle_pushover(conn, ids: &[i64]) -> Result<(), String>`
  - `pub const PUSHOVER_RETRY_DAYS: i64 = 2;`
- Produces（lib.rs command）: `pushover_status() -> PushoverStatus`；`send_test_notification() -> TestNotifyOutcome { desktop_ok, desktop_error, pushover: Option<PushoverTestResult{ok,error}> }`

- [ ] **Step 3.1: 写失败测试（pushover 纯逻辑）**

pushover.rs 底部（实现前先建空模块 + lib.rs 挂 `mod pushover;`，让测试跑起来红）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_with_builds_form_with_credentials_and_text() {
        let cfg = PushoverConfig { user: "u-测试用户".into(), token: "t-令牌".into() };
        let mut seen: Option<(String, Form)> = None;
        send_with(&cfg, "该喂面包虫了", "「大头一号」已 7 天没喂面包虫", &|url, form| {
            seen = Some((url.to_string(), form.clone()));
            Ok(())
        })
        .unwrap();
        let (url, form) = seen.expect("传输层应被调用一次");
        assert_eq!(url, API_URL);
        let get = |k: &str| form.iter().find(|(key, _)| key == k).unwrap().1.clone();
        assert_eq!(get("token"), "t-令牌");
        assert_eq!(get("user"), "u-测试用户");
        assert_eq!(get("title"), "该喂面包虫了");
        assert_eq!(get("message"), "「大头一号」已 7 天没喂面包虫");
    }

    #[test]
    fn send_with_propagates_transport_error() {
        let cfg = PushoverConfig { user: "u".into(), token: "t".into() };
        let err = send_with(&cfg, "标题", "正文", &|_, _| Err("DNS 解析失败".into())).unwrap_err();
        assert!(err.contains("DNS"), "实际错误：{err}");
    }
}
```

（`from_env`/`status_from_env` 涉及进程级环境变量，单测并行不安全——真机验证走「发送测试通知」按钮，不写 set_var 测试。）

- [ ] **Step 3.2: 确认失败 → 实现 pushover.rs**

Run: `cargo test --manifest-path src-tauri/Cargo.toml pushover::` → FAIL，然后实现：

```rust
//! Pushover 手机推送（反馈第二轮 F4，Q3/Q4/Q10）：凭证只读环境变量，不进库不进备份。
//! HTTP 经 `post` 参数注入（cargo test 不真发）；真实发送用 ureq（阻塞式，调度线程可承受）。

use serde::Serialize;

pub const API_URL: &str = "https://api.pushover.net/1/messages.json";
pub const ENV_USER: &str = "PUSHOVER_USER";
pub const ENV_TOKEN: &str = "PUSHOVER_TOKEN";
pub const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct PushoverConfig {
    pub user: String,
    pub token: String,
}

impl PushoverConfig {
    /// 两个变量都读到非空值才算已配置；缺任一 → None（未配置，不是错误）。
    pub fn from_env() -> Option<PushoverConfig> {
        let user = std::env::var(ENV_USER).ok()?.trim().to_string();
        let token = std::env::var(ENV_TOKEN).ok()?.trim().to_string();
        if user.is_empty() || token.is_empty() {
            return None;
        }
        Some(PushoverConfig { user, token })
    }
}

pub type Form = Vec<(String, String)>;

/// 组表单经注入的传输层发送；错只在传输层。
pub fn send_with(
    cfg: &PushoverConfig,
    title: &str,
    message: &str,
    post: &dyn Fn(&str, &Form) -> Result<(), String>,
) -> Result<(), String> {
    let form: Form = vec![
        ("token".into(), cfg.token.clone()),
        ("user".into(), cfg.user.clone()),
        ("title".into(), title.to_string()),
        ("message".into(), message.to_string()),
    ];
    post(API_URL, &form)
}

/// 真实发送（ureq 负责表单 percent-encoding，中文标题/正文安全）。
pub fn send(cfg: &PushoverConfig, title: &str, message: &str) -> Result<(), String> {
    send_with(cfg, title, message, &|url, form| {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .build();
        match agent.post(url).send_form(form.iter().cloned()) {
            Ok(resp) if resp.status() < 300 => Ok(()),
            Ok(resp) => Err(format!("Pushover 返回 HTTP {}", resp.status())),
            Err(e) => Err(format!("Pushover 发送失败: {e}")),
        }
    })
}

/// 设置页状态探测：只报在/不在，不回报值。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PushoverStatus {
    pub user_found: bool,
    pub token_found: bool,
}

pub fn status_from_env() -> PushoverStatus {
    let found = |k: &str| std::env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false);
    PushoverStatus { user_found: found(ENV_USER), token_found: found(ENV_TOKEN) }
}
```

Cargo.toml `[dependencies]` 加 `ureq = "2"`（先 `cargo build` 拉依赖）。跑 pushover 测试转绿。

- [ ] **Step 3.3: 写失败测试（v5 迁移）**

db.rs tests（版本断言改 5）：

```rust
#[test]
fn v5_adds_push_columns_and_settles_legacy_rows() {
    let conn = Connection::open_in_memory().expect("内存库打开失败");
    conn.execute_batch(V1_SCHEMA_SQL).unwrap();
    seed_v1_presets(&conn).unwrap();
    migrate_v1_to_v2(&conn).unwrap();
    migrate_v2_to_v3(&conn).unwrap();
    conn.execute("INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')", []).unwrap();
    conn.execute(
        "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at)
         VALUES (1, 'overdue', 1, '2026-09-15', '2026-09-15 08:00:00')",
        [],
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 4).unwrap(); // F2 之后真实库至少是 v4

    migrate(&conn).unwrap();
    // 历史行只走过桌面通道：直接视为已了结，不参与补发
    let done: i64 = conn.query_row("SELECT pushover_done FROM reminder_ledger", [], |r| r.get(0)).unwrap();
    assert_eq!(done, 1);
    // 新列存在且可为空
    let (title, body): (Option<String>, Option<String>) = conn
        .query_row("SELECT push_title, push_body FROM reminder_ledger", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((title, body), (None, None));
}
```

- [ ] **Step 3.4: 实现 v5**

```rust
/// v5 → v?（F4）：台账加推送列。push_title/push_body = 发送当时的通知文案快照
/// （补发时直接用，不重算）；pushover_done = 手机侧已了结（已送达或超窗放弃）。
/// 历史行升级即了结（它们的桌面通知在当天已发过，不补手机）。
fn migrate_v4_to_v5(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE reminder_ledger ADD COLUMN push_title TEXT;
        ALTER TABLE reminder_ledger ADD COLUMN push_body TEXT;
        ALTER TABLE reminder_ledger ADD COLUMN pushover_done INTEGER NOT NULL DEFAULT 0;
        UPDATE reminder_ledger SET pushover_done = 1;
        "#,
    )?;
    tx.pragma_update(None, "user_version", 5)?;
    tx.commit()
}
```

（`SCHEMA_VERSION = 5`，match 加 `4 => migrate_v4_to_v5(conn)?,`。）

- [ ] **Step 3.5: 写失败测试（run_check 拆 IO + 补发）**

reminder.rs tests 改造要点（既有 ~10 处 `run_check(...).unwrap()` 全部改 `.unwrap().toasts`——机械替换）+ 新增：

```rust
#[test]
fn pushover_failures_retry_next_round_and_settle_on_success() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号", "active");
    feed(&conn, c, "喂食", "2026-09-10 08:00:00"); // 超期

    // 第一轮：新插入 → 1 条 toast + 1 个待发推送任务（发送在锁外，由调用方做）
    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert_eq!(out.toasts.len(), 1);
    assert_eq!(out.push_jobs.len(), 1);
    assert_eq!(out.push_jobs[0].title, "喂食超期");
    // 模拟发送失败：不 settle

    // 同日第二轮：toast 不重发，推送任务仍在（补发）
    let out = run_check(&conn, TODAY, "2026-09-18 12:00:00").unwrap();
    assert!(out.toasts.is_empty());
    assert_eq!(out.push_jobs.len(), 1);
    let id = out.push_jobs[0].ledger_id;

    // 发送成功 → settle → 第三轮无任务
    settle_pushover(&conn, &[id]).unwrap();
    let out = run_check(&conn, TODAY, "2026-09-18 18:00:00").unwrap();
    assert!(out.push_jobs.is_empty());
}

#[test]
fn stale_unsettled_push_jobs_are_abandoned_beyond_retry_window() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号", "active");
    feed(&conn, c, "喂食", "2026-09-01 08:00:00");
    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert_eq!(out.push_jobs.len(), 1);
    // 伪造：该行三天前就登记且一直没发成功
    conn.execute(
        "UPDATE reminder_ledger SET sent_at = '2026-09-14 08:00:00' WHERE id = ?1",
        params![out.push_jobs[0].ledger_id],
    )
    .unwrap();
    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert!(out.push_jobs.is_empty(), "超窗不再补发");
    let done: i64 = conn
        .query_row("SELECT pushover_done FROM reminder_ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(done, 1, "超窗自动了结");
}

#[test]
fn category_switches_no_longer_filter_since_feedback2() {
    // Q7/Q9：分类子开关作废——关着也照发（总开关才是唯一闸门）
    let conn = mem_conn();
    let a = colony(&conn, "活跃一号", "active");
    let h = colony(&conn, "冬眠一号", "hibernating");
    feed(&conn, a, "喂食", "2026-09-10 08:00:00");
    open_seg(&conn, h, TODAY);
    settings::set_settings(
        &conn,
        &AppSettings { notify_overdue_enabled: false, notify_hibernation_enabled: false, ..Default::default() },
    )
    .unwrap();
    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert_eq!(out.toasts.len(), 3, "超期 1 + 临近/出眠日 2，分类开关不再过滤");
}
```

（既有 `master_switch_off...` 测试同步改 `.toasts`；`category_switches_filter_by_kind` 删除，由上面新测试取代。）

- [ ] **Step 3.6: 实现 run_check 改造**

reminder.rs：

```rust
/// 手机推送补发窗口（天）：登记起两天内没发成功就放弃（Q8=A，超窗防陈年补发）。
pub const PUSHOVER_RETRY_DAYS: i64 = 2;

/// 一个待发推送任务：IO 在锁外做，成功后拿 ledger_id 去 settle。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PushJob {
    pub ledger_id: i64,
    pub title: String,
    pub body: String,
}

/// run_check 返回体：toasts = 本轮新登记（该发桌面通知）；push_jobs = 待发/补发的
/// 手机推送（新登记 + 窗口内未了结）。发送与了结都在锁外由调用方驱动。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    pub toasts: Vec<Reminder>,
    pub push_jobs: Vec<PushJob>,
}
```

`run_check` 新体（替换 213-247 行）：master 判断后**删掉分类 filter**；INSERT 语句加 push_title/push_body 两列（值来自 `r.notification_text()`）；inserted==1 时 push toasts 并把 `(row_id, title, body)` 存入 push_jobs（`tx.last_insert_rowid()`）；事务提交后跑补发扫描：

```rust
fn collect_retry_jobs(conn: &Connection, today: &str) -> Result<Vec<PushJob>, String> {
    // 先了结超窗/无文案的历史行，再捞窗口内未了结的
    conn.execute(
        "UPDATE reminder_ledger SET pushover_done = 1
         WHERE pushover_done = 0
           AND date(sent_at) < date(?1, ?2)",
        params![today, format!("-{PUSHOVER_RETRY_DAYS} day")],
    )
    .map_err(db_err)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, push_title, push_body FROM reminder_ledger
             WHERE pushover_done = 0 AND push_title IS NOT NULL",
        )
        .map_err(db_err)?;
    let jobs = stmt
        .query_map([], |row| {
            Ok(PushJob {
                ledger_id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(jobs)
}

/// 手机侧了结（送达或放弃）——由调用方在发送拿到结果后调用。
pub fn settle_pushover(conn: &Connection, ids: &[i64]) -> Result<(), String> {
    for id in ids {
        conn.execute("UPDATE reminder_ledger SET pushover_done = 1 WHERE id = ?1", params![id])
            .map_err(db_err)?;
    }
    Ok(())
}
```

（窗口判断也可在 SELECT 里做——上面「先 UPDATE 了结再全捞」语义等价且简单；`date(sent_at)` 对 `YYYY-MM-DD HH:MM:SS` 前缀生效。）

`check_and_notify` 改为（IO 全在锁外）：

```rust
pub fn check_and_notify(handle: &tauri::AppHandle) {
    let Some(state) = handle.try_state::<crate::DbState>() else { return };
    let today = crate::colony::today_iso();
    let now = crate::care::now_local();
    let outcome = {
        let Ok(conn) = state.0.lock() else { return };
        match run_check(&conn, &today, &now) {
            Ok(outcome) => outcome,
            Err(_) => return,
        }
    };
    for r in &outcome.toasts {
        send_notification(handle, r);
    }
    if !outcome.push_jobs.is_empty() {
        let cfg = crate::pushover::PushoverConfig::from_env();
        let mut settled: Vec<i64> = Vec::new();
        for job in &outcome.push_jobs {
            if let Some(cfg) = &cfg {
                match crate::pushover::send(cfg, &job.title, &job.body) {
                    Ok(()) => settled.push(job.ledger_id),
                    Err(e) => eprintln!("[pushover] 发送失败（下轮重试）: {e}"),
                }
            }
        }
        if !settled.is_empty() {
            if let Ok(conn) = state.0.lock() {
                let _ = settle_pushover(&conn, &settled);
            }
        }
    }
    refresh_tray_tooltip(handle);
}
```

（未配置 = 不发送也不了结——每轮空转一次、零网络调用，用户当天配好环境变量并重启后仍能收到当日补发；超窗自动放弃兜底。）

- [ ] **Step 3.7: 测试通知返回结构 + pushover_status 命令**

reminder.rs 加：

```rust
/// 测试通知结果（分渠道回显；pushover=None 表示未配置）。
#[derive(Debug, Clone, Serialize)]
pub struct PushoverTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestNotifyOutcome {
    pub desktop_ok: bool,
    pub desktop_error: Option<String>,
    pub pushover: Option<PushoverTestResult>,
}

pub fn send_test_notification_dual(handle: &tauri::AppHandle) -> TestNotifyOutcome {
    let desktop = handle
        .notification()
        .builder()
        .title("测试通知")
        .body("蚂蚁饲养记录：桌面通道正常。")
        .show()
        .map_err(|e| format!("发送测试通知失败: {e}"));
    let pushover = crate::pushover::PushoverConfig::from_env().map(|cfg| {
        match crate::pushover::send(&cfg, "测试通知", "蚂蚁饲养记录：手机通道正常。") {
            Ok(()) => PushoverTestResult { ok: true, error: None },
            Err(e) => PushoverTestResult { ok: false, error: Some(e) },
        }
    });
    TestNotifyOutcome {
        desktop_ok: desktop.is_ok(),
        desktop_error: desktop.err(),
        pushover,
    }
}
```

lib.rs：`mod pushover;`；`send_test_notification` 改为调 `send_test_notification_dual` 返回 `reminder::TestNotifyOutcome`；新命令：

```rust
#[tauri::command]
fn pushover_status() -> pushover::PushoverStatus {
    pushover::status_from_env()
}
```

`generate_handler!` 加 `pushover_status`。旧 `send_test_notification` 函数体删除。

- [ ] **Step 3.8: 前端——通知 tab 重排**

- `src/types.ts` 加：

```ts
/** pushover_status 返回体：环境变量在/不在（不含值） */
export interface PushoverStatus {
  user_found: boolean;
  token_found: boolean;
}

/** send_test_notification 返回体：分渠道结果（pushover=null 表示未配置） */
export interface PushoverTestResult { ok: boolean; error: string | null; }
export interface TestNotifyOutcome {
  desktop_ok: boolean;
  desktop_error: string | null;
  pushover: PushoverTestResult | null;
}
```

- `notifySettings.ts`：`NotifySettingsForm` 删 `overdue/hibernation`，留 `{ master, daysAheadText }`；`toForm` 同步；`toSettings` 固定写 `notify_overdue_enabled: true, notify_hibernation_enabled: true`（键仍在库里，行为已由总开关统一——见共识文档）。notifySettings.test.ts 同步改。

- `SettingsDialog.vue` 通知 tab：
  - `load()` 里加 `invoke<PushoverStatus>("pushover_status")` 存 `pushoverStatus`。
  - 删两个分类子开关的 `.notify-row`（419-425 行）；master 文案改「推送通知（桌面 + 手机）」。
  - 状态区 + 测试按钮改：

```html
<div class="notify-row">
  <span>手机推送（Pushover）：</span>
  <span v-if="pushoverStatus?.user_found && pushoverStatus?.token_found" class="push-ok">
    已配置（环境变量 PUSHOVER_USER / PUSHOVER_TOKEN）
  </span>
  <span v-else class="push-miss">
    未检测到（需设置环境变量 PUSHOVER_USER / PUSHOVER_TOKEN，配置后重启应用生效）
  </span>
</div>
```

```ts
const pushoverStatus = ref<PushoverStatus | null>(null);

async function testNotify() {
  notifyError.value = "";
  notifySaved.value = "";
  try {
    const r = await invoke<TestNotifyOutcome>("send_test_notification");
    const parts = [
      r.desktop_ok ? "桌面 ✓" : `桌面 ✗（${r.desktop_error ?? "未知错误"}）`,
      r.pushover === null ? "手机：未配置" : r.pushover.ok ? "手机 ✓" : `手机 ✗（${r.pushover.error}）`,
    ];
    notifySaved.value = `测试结果：${parts.join(" · ")}`;
  } catch (e) {
    notifyError.value = String(e);
  }
}
```

- 样式加 `.push-ok { color: var(--ok, #2e7d32); } .push-miss { color: var(--bad); }`。
- App.test.ts 夹具/断言涉及 `notifyForm.overdue` 等的用例同步机械修改（grep `overdue: true` 定位）。

- [ ] **Step 3.9: 全量验证 + 提交**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` + `pnpm test` + `pnpm build` + `cargo build --manifest-path src-tauri/Cargo.toml`（确认 ureq 编译）。

```bash
git add -A src-tauri src/types.ts src/lib/notifySettings.ts src/lib/notifySettings.test.ts src/components/SettingsDialog.vue src/App.test.ts
git commit -m "feat(notify): 通知双通道——Pushover 读环境变量+断网补发+单总开关（票 F4，v5）"
```

---

### Task 4（票 F3）：双层喂食周期

**Files:**
- Modify: `src-tauri/src/db.rs`（v6：food 周期列 + 台账整表重建加 food 维度）
- Modify: `src-tauri/src/care.rs`（FoodTileStatus、tiles 喂食明细）
- Modify: `src-tauri/src/dict.rs`（FoodInput 周期 + save_food）
- Modify: `src-tauri/src/reminder.rs`（FoodOverdue 种类 + 台账 food_id）
- Modify: `src/types.ts`、`src/lib/care.ts`、`src/lib/dict.ts`、`src/components/ColonyCard.vue`、`src/components/SettingsDialog.vue`
- Test: 上述各 Rust 模块内嵌 tests + `src/lib/care.test.ts`、`src/lib/dict.test.ts`、`src/App.test.ts`

**Interfaces:**
- Produces（Rust）:
  - `care::FoodTileStatus { food_id, name, suggested_interval_days, days_since_last, overdue }`；`ActionTile.foods: Vec<FoodTileStatus>`（非喂食为空）。
  - `care::Food.suggested_interval_days: Option<i64>`；`dict::FoodInput.suggested_interval_days: Option<i64>`。
  - `ReminderKind::FoodOverdue`（"food_overdue"）；`Reminder.food_id: Option<i64>`、`food_name: Option<String>`。
- Produces（前端）: `FoodTileInfo`（同 FoodTileStatus）、`ColonyAction.foods`、`FoodItem.suggested_interval_days`、`FoodInput.suggested_interval_days`、`FoodRow.intervalText`、`feedingTooltip(foods): string`。

- [ ] **Step 4.1: 写失败测试（v6 迁移）**

db.rs tests（版本断言改 6）：

```rust
#[test]
fn v6_adds_food_interval_and_rebuilds_ledger_with_food_dimension() {
    let conn = Connection::open_in_memory().expect("内存库打开失败");
    conn.execute_batch(V1_SCHEMA_SQL).unwrap();
    seed_v1_presets(&conn).unwrap();
    migrate_v1_to_v2(&conn).unwrap();
    migrate_v2_to_v3(&conn).unwrap();
    conn.pragma_update(None, "user_version", 4).unwrap();
    // v5 列（F4 之后真实库至少 v5；这里手工补齐等价结构再升 v6）
    conn.execute_batch(
        "ALTER TABLE reminder_ledger ADD COLUMN push_title TEXT;
         ALTER TABLE reminder_ledger ADD COLUMN push_body TEXT;
         ALTER TABLE reminder_ledger ADD COLUMN pushover_done INTEGER NOT NULL DEFAULT 0;",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 5).unwrap();
    conn.execute(
        "INSERT INTO colony (name, start_date) VALUES ('大头一号', '2026-01-20')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO reminder_ledger (colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done)
         VALUES (1, 'overdue', 1, '2026-09-15', '2026-09-15 08:00:00', '喂食超期', '正文', 1)",
        [],
    )
    .unwrap();

    migrate(&conn).unwrap();

    // 食物周期：预置三样按名回填，自建为 NULL
    let intervals: Vec<(String, Option<i64>)> = {
        let mut stmt = conn.prepare("SELECT name, suggested_interval_days FROM food ORDER BY sort").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(intervals, vec![
        ("种子".into(), Some(3)), ("干虾仁".into(), Some(7)), ("面包虫".into(), Some(7)),
    ]);

    // 台账重建后：旧行与推送列原样保留 + 新 food_id 列（NULL）
    let row: (Option<i64>, Option<String>, i64) = conn
        .query_row("SELECT food_id, push_title, pushover_done FROM reminder_ledger", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(row, (None, Some("喂食超期".into()), 1));

    // food_overdue 维度的唯一键生效：同窝同操作同日、不同食物共存；同食物重复被拒
    let insert = |food: Option<i64>| {
        conn.execute(
            "INSERT INTO reminder_ledger (colony_id, kind, action_id, food_id, base_date, sent_at)
             VALUES (1, 'food_overdue', 1, ?1, '2026-09-15', '2026-09-15 08:00:00')",
            params![food],
        )
    };
    assert_eq!(insert(Some(1)).unwrap(), 1);
    assert_eq!(insert(Some(2)).unwrap(), 1);
    assert!(insert(Some(1)).is_err());
}
```

- [ ] **Step 4.2: 实现 v6**

```rust
/// v6（F3）：① food 加 suggested_interval_days（按名回填 种子3/干虾仁7/面包虫7）；
/// ② reminder_ledger 整表重建——v1 的 kind CHECK 不含 'food_overdue' 且 SQLite 不能
/// 改列约束，顺带加 food_id 维度；kind 合法性改由应用层（compute 只产四种）保证。
/// 数据、推送列（v5）、既有唯一键语义全部原样保留。
fn migrate_v5_to_v6(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE food ADD COLUMN suggested_interval_days INTEGER;
        UPDATE food SET suggested_interval_days = CASE name
            WHEN '种子' THEN 3
            WHEN '干虾仁' THEN 7
            WHEN '面包虫' THEN 7
        END
        WHERE name IN ('种子', '干虾仁', '面包虫');

        CREATE TABLE reminder_ledger_v6 (
            id             INTEGER PRIMARY KEY,
            colony_id      INTEGER NOT NULL REFERENCES colony(id),
            kind           TEXT    NOT NULL,
            action_id      INTEGER REFERENCES care_action(id),
            food_id        INTEGER REFERENCES food(id),
            base_date      TEXT    NOT NULL,
            sent_at        TEXT    NOT NULL,
            push_title     TEXT,
            push_body      TEXT,
            pushover_done  INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO reminder_ledger_v6
            (id, colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done)
            SELECT id, colony_id, kind, action_id, base_date, sent_at, push_title, push_body, pushover_done
            FROM reminder_ledger;
        DROP TABLE reminder_ledger;
        ALTER TABLE reminder_ledger_v6 RENAME TO reminder_ledger;

        CREATE UNIQUE INDEX uq_ledger_overdue
            ON reminder_ledger(colony_id, action_id, base_date) WHERE kind = 'overdue';
        CREATE UNIQUE INDEX uq_ledger_food
            ON reminder_ledger(colony_id, action_id, food_id, base_date) WHERE kind = 'food_overdue';
        CREATE UNIQUE INDEX uq_ledger_wake
            ON reminder_ledger(colony_id, kind, base_date)
            WHERE kind IN ('approaching_wake', 'wake_day');
        "#,
    )?;
    tx.pragma_update(None, "user_version", 6)?;
    tx.commit()
}
```

（`SCHEMA_VERSION = 6`，match 加 `5 => migrate_v5_to_v6(conn)?,`。）

- [ ] **Step 4.3: 写失败测试（care：tile 食物明细）**

care.rs tests 新增：

```rust
fn feed_log_with(conn: &Connection, c: i64, at: &str, foods: &[&str]) -> i64 {
    log_care(conn, &CareLogInput {
        colony_id: c,
        action_id: action_id(conn, "喂食"),
        happened_at: at.into(),
        note: None,
        food_ids: foods.iter().map(|f| food_id(conn, f)).collect(),
    }, NOW_FOR_LOG)
    .unwrap()
}
const NOW_FOR_LOG: &str = "2026-09-18 08:00:00";

#[test]
fn feeding_tile_lists_per_food_days_and_flags_food_overdue() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号");
    // 2 天前喂了种子；8 天前喂过面包虫（周期 7 → 面包虫超期）
    feed_log_with(&conn, c, "2026-09-16 20:00:00", &["种子"]);
    feed_log_with(&conn, c, "2026-09-10 20:00:00", &["面包虫"]);

    let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
    let feed = tile(&tiles, "喂食");
    assert!(!feed.overdue, "统一层：距上次任何喂食 2 天 ≤ 3，不红");
    assert_eq!(feed.days_since_last, Some(2));

    let foods: Vec<(String, Option<i64>, bool)> = feed.foods.iter()
        .map(|f| (f.name.clone(), f.days_since_last, f.overdue)).collect();
    assert_eq!(foods, vec![
        ("种子".into(), Some(2), false),
        ("干虾仁".into(), None, false),   // 从未喂过且设了周期 → None 不超期（同"从未记录"口径）
        ("面包虫".into(), Some(8), true), // 8 > 7 → 食物层超期
    ]);

    // 非喂食 tile 不带食物明细
    assert!(tile(&tiles, "垃圾清理").foods.is_empty());
}

#[test]
fn feeding_tile_red_when_any_food_overdue_even_if_operation_layer_fresh() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号");
    feed_log_with(&conn, c, "2026-09-16 20:00:00", &["种子"]);
    feed_log_with(&conn, c, "2026-09-10 20:00:00", &["面包虫"]);
    let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
    assert!(tile(&tiles, "喂食").overdue, "任一层超期即红（Q2 决议）");
}

#[test]
fn food_without_interval_never_flags_and_wake_resets_food_clock() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号");
    conn.execute("UPDATE food SET suggested_interval_days = NULL WHERE name = '种子'", []).unwrap();
    feed_log_with(&conn, c, "2026-08-01 20:00:00", &["种子"]); // 48 天前
    let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
    let seed = tile(&tiles, "喂食").foods.iter().find(|f| f.name == "种子").unwrap();
    assert!(!seed.overdue, "未设周期只受统一周期管（统一层 48>3 会红，但食物项自身不标");

    // 出眠基线同样作用于食物层：出眠当天全部归零
    conn.execute(
        "INSERT INTO hibernation (colony_id, start_date, expected_end_date, actual_end_date)
         VALUES (?1, '2026-09-10', '2026-10-01', '2026-09-18')",
        params![c],
    )
    .unwrap();
    let tiles = tiles_for_colony(&conn, c, TODAY).unwrap();
    let worm = tile(&tiles, "喂食").foods.iter().find(|f| f.name == "面包虫").unwrap();
    assert_eq!(worm.days_since_last, Some(0), "从未喂过的食物从出眠日起算");
}
```

注意 `feed_log_with` 与既有 `feed_log` helper 同名冲突——直接扩展现有 `feed_log`（它已接受 foods: &[&str]）即可，无需新 helper。

- [ ] **Step 4.4: 实现 tiles 食物明细（care.rs）**

- `Food` DTO/`FOOD_SQL`/`row_to_food` 加 `suggested_interval_days`（SQL 列序：id, name, enabled, sort, suggested_interval_days, is_preset, EXISTS → 索引 0-6）。
- 新 DTO `FoodTileStatus`（见 Interfaces）。
- `ActionTile` 加 `pub foods: Vec<FoodTileStatus>,`。
- `tiles_for_colony` 喂食分支：查启用食物（含周期），逐食物查该窝该食物的最近喂食日期：

```rust
let food_rows: Vec<(i64, String, Option<i64>)> = { /* SELECT id, name, suggested_interval_days FROM food WHERE enabled = 1 ORDER BY sort, id */ };
let mut foods = Vec::with_capacity(food_rows.len());
for (food_id, food_name, food_interval) in food_rows {
    let occurred: Vec<String> = { /* SELECT l.occurred_at FROM log_food lf JOIN care_log l ON l.id = lf.log_id JOIN care_action a ON a.id = l.action_id WHERE lf.food_id = ?1 AND l.colony_id = ?2 AND a.is_feeding = 1 */ };
    let latest = occurred.iter()
        .filter_map(|s| chrono::NaiveDate::parse_from_str(s.get(0..10).unwrap_or(""), "%Y-%m-%d").ok())
        .max();
    let base = match (latest, wake) {
        (Some(log), Some(w)) => Some(log.max(w)),
        (log, w) => log.or(w),
    };
    let days = days_since_last(base.map(|d| d.format("%Y-%m-%d").to_string()).as_deref(), today)?;
    let food_overdue = if kind == "reminding" { is_overdue("reminding", days, food_interval) } else { false };
    foods.push(FoodTileStatus { food_id, name: food_name, suggested_interval_days: food_interval, days_since_last: days, overdue: food_overdue });
}
let food_any = foods.iter().any(|f| f.overdue);
// overdue = is_overdue(操作层) || food_any；非喂食 foods 为空 vec
```

- 既有 tiles 测试的断言不动（非喂食 foods 空、overdue 语义不变）；`ActionTile` 字面量构造处（reminder.rs tests 的 `tile()` helper）补 `foods: vec![]`。

- [ ] **Step 4.5: 写失败测试（reminder：FoodOverdue）**

reminder.rs tests 新增（`feed` helper 扩成带食物版本，参考 dict.rs 的 `log_feeding`）：

```rust
#[test]
fn food_overdue_fires_independently_of_operation_layer() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号", "active");
    // 只喂种子（2 天前）：统一层不超期；面包虫 8 天前喂过 → 食物层超期
    feed_foods(&conn, c, "2026-09-16 20:00:00", &["种子"]);
    feed_foods(&conn, c, "2026-09-10 20:00:00", &["面包虫"]);

    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert_eq!(out.toasts.len(), 1, "只有面包虫食物层一条");
    let r = &out.toasts[0];
    assert_eq!(r.kind, ReminderKind::FoodOverdue);
    assert_eq!(r.action_name.as_deref(), Some("喂食"));
    assert_eq!(r.food_name.as_deref(), Some("面包虫"));
    assert_eq!(r.days_since_last, Some(8));
    assert_eq!(r.base_date, fmt(day(TODAY) - Duration::days(7)));
    let (title, body) = r.notification_text();
    assert_eq!(title, "该喂面包虫了");
    assert_eq!(body, "「大头一号」已 8 天没喂面包虫（建议 7 天一次）");

    // 同日重查不重发；台账 food 维度去重
    assert!(run_check(&conn, TODAY, "2026-09-18 12:00:00").unwrap().toasts.is_empty());
    let kinds: Vec<(String, Option<i64>)> = ledger_rows(&conn); // 扩展 helper 带 food_id
    assert_eq!(kinds.len(), 1);
}

#[test]
fn operation_and_food_layers_same_day_both_fire() {
    let conn = mem_conn();
    let c = colony(&conn, "大头一号", "active");
    feed_foods(&conn, c, "2026-09-10 20:00:00", &["种子", "面包虫"]); // 8 天 → 两层全超
    let out = run_check(&conn, TODAY, NOW).unwrap();
    assert_eq!(out.toasts.len(), 3, "统一层 1 + 种子 1 + 面包虫 1，各记各的");
}

#[test]
fn hibernating_colony_food_layer_silent_too() {
    let conn = mem_conn();
    let h = colony(&conn, "冬眠一号", "hibernating");
    open_seg(&conn, h, "2027-03-01");
    feed_foods(&conn, h, "2026-06-01 08:00:00", &["面包虫"]);
    assert!(run_check(&conn, TODAY, NOW).unwrap().toasts.is_empty());
}
```

（`ledger_rows` helper 扩为返回 `(kind, action_id, base_date, food_id)`。）

- [ ] **Step 4.6: 实现 reminder FoodOverdue**

- `ReminderKind` 加 `FoodOverdue`（`as_str` → "food_overdue"；serde snake_case 自动）。
- `Reminder` 加 `food_id: Option<i64>`、`food_name: Option<String>`（doc：仅食物层非空）。既有测试的字面量构造补 `food_id: None, food_name: None`。
- `compute_due_reminders` active 分支：现有 overdue push 不动；其后加：

```rust
for f in tile.foods.iter().filter(|f| f.overdue) {
    let Some(interval) = f.suggested_interval_days else { continue };
    let base = (today - Duration::days(interval.max(0))).format("%Y-%m-%d").to_string();
    due.push(Reminder {
        colony_id,
        colony_name: colony_name.clone(),
        kind: ReminderKind::FoodOverdue,
        action_id: Some(tile.action_id),
        action_name: Some(tile.name.clone()),
        food_id: Some(f.food_id),
        food_name: Some(f.name.clone()),
        base_date: base,
        days_since_last: f.days_since_last,
        suggested_interval_days: f.suggested_interval_days,
    });
}
```

（外层循环条件从 `.filter(|t| t.overdue)` 改为 `.filter(|t| t.overdue || t.is_feeding)`——喂食 tile 即使统一层不红也要扫食物层。）
- `notification_text` 加分支：

```rust
ReminderKind::FoodOverdue => {
    let food = self.food_name.as_deref().unwrap_or("食物");
    let action = self.action_name.as_deref().unwrap_or("喂食");
    let mut body = format!("「{}」已 ", self.colony_name);
    match self.days_since_last {
        Some(d) => body.push_str(&format!("{d} 天")),
        None => body.push_str("有一阵子"),
    }
    body.push_str(&format!("没{action}{food}"));
    if let Some(n) = self.suggested_interval_days {
        body.push_str(&format!("（建议 {n} 天一次）"));
    }
    (format!("该喂{food}了"), body)
}
```

- `run_check` INSERT 加 `food_id` 列（`r.food_id`）。
- `tray_summary`：超期行若带食物明细，改报食物（`for f in t.foods.iter().filter(|f| f.overdue) { parts.push(format!("{}该喂{}了", c.name, f.name)); }`，统一层超期维持原句式；两者可并存）。对应 tray 测试的 `tile()` helper 夹具加 `foods: vec![]`，并补一个带食物超期的断言用例。

- [ ] **Step 4.7: dict.rs FoodInput 周期 + 前端设置食物 tab**

Rust：
- `FoodInput` 加 `pub suggested_interval_days: Option<i64>,`（`#[serde(default)]` 兼容旧调用）。
- `save_food`：开头 `validate_interval(input.suggested_interval_days)?`；INSERT 列加 `suggested_interval_days`；UPDATE SET 加同列。既有 dict tests 的 `FoodInput` 构造补 `suggested_interval_days: None`；新增：

```rust
#[test]
fn save_food_sets_and_clears_interval() {
    let conn = mem_conn();
    let seed = food_id(&conn, "种子");
    save_food(&conn, &FoodInput { id: Some(seed), name: "种子".into(), sort: 1, suggested_interval_days: Some(5) }).unwrap();
    assert_eq!(crate::care::list_foods(&conn).unwrap().iter().find(|f| f.id == seed).unwrap().suggested_interval_days, Some(5));
    save_food(&conn, &FoodInput { id: Some(seed), name: "种子".into(), sort: 1, suggested_interval_days: None }).unwrap();
    assert_eq!(crate::care::list_foods(&conn).unwrap().iter().find(|f| f.id == seed).unwrap().suggested_interval_days, None);
    assert!(save_food(&conn, &FoodInput { id: None, name: "糖水".into(), sort: 9, suggested_interval_days: Some(0) })
        .unwrap_err().contains("建议间隔"));
}
```

前端：
- `types.ts`：`FoodItem`/`FoodInput` 加 `suggested_interval_days: number | null`；`FoodTileInfo` 新接口 + `ColonyAction.foods: FoodTileInfo[]`。
- `dict.ts`：`FoodRow` 加 `intervalText: string | number`（空串 = 不设）；`buildFoodRows` 映射；`toFoodInputs` 用既有 `parseInterval`；`validateFoodRows` 加与操作同款的间隔校验（文案「食物「x」的建议间隔应是不小于 1 的整数天数」）。
- `care.ts` 加：

```ts
/** 喂食 tile 悬停提示：逐食物"距上次"，超期的标出来。 */
export function feedingTooltip(foods: FoodTileInfo[]): string {
  return foods
    .map((f) => {
      const days = f.days_since_last === null ? "尚未记录" : `距上次 ${f.days_since_last} 天`;
      const mark = f.overdue ? " · 超期" : "";
      return `${f.name}：${days}${mark}`;
    })
    .join("\n");
}
```

（care.test.ts 补 feedingTooltip 三态断言。）
- `ColonyCard.vue` tile 按钮：`:title="a.is_feeding && a.foods.length > 0 ? feedingTooltip(a.foods) : undefined"`；import `feedingTooltip`。测试夹具（App.test.ts 的 `waterReg` 等）补 `foods: []`；新增一个带食物明细的红态+title 断言用例。
- `SettingsDialog.vue` 食物 tab：名字输入后加

```html
<input
  v-model="row.intervalText"
  class="interval-input"
  type="number"
  min="1"
  title="食物建议间隔：距上次喂该食物超过它就单独提醒；留空 = 只按喂食统一周期"
/>
```

食物 tab hint 补一句：「设了间隔的食物各自算"距上次"，任一超期喂食块就变红并单独提醒。」

- [ ] **Step 4.8: 全量验证 + 提交**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` + `pnpm test` + `pnpm build`。

```bash
git add -A
git commit -m "feat(feeding): 双层周期——食物各自建议间隔/距上次/超期/提醒（票 F3，v6 台账加食物维度）"
```

---

## Task 5：真机验收（全部票完成后）

`pnpm tauri dev` 起真应用，人工过一遍（自动化不覆盖的系统行为）：

- [ ] 三个非喂食操作弹面板：取消零落库；补录昨天生效；备注可见于记录页。
- [ ] 设置里默认四操作/三食物删除按钮灰、悬停有文案；改名/停用正常。
- [ ] 食物 tab 改种子周期为 2 → 首页喂食块 tooltip 与红态跟着变。
- [ ] 通知 tab：只剩一个总开关；Pushover 状态区正确显示；「发送测试通知」桌面弹 + 手机各一条、分渠道回显。
- [ ] 断网（或临时改错 PUSHOVER_TOKEN）→ 桌面通知照发；恢复后 ≤30 分钟手机收到补发。
- [ ] 旧库升级：拿一份 v3 版本的 .db 打开 → 数据完好、预置保护与食物周期生效。

## Self-Review 记录

- 决议覆盖：Q1→Task1；Q2→Task4；Q3/Q4/Q7/Q9/Q10/Q8→Task3；Q5→Task2；Q6（不加删除入口）→Global Constraints 排除项。共识文档排除项均未实现。
- 版本链：v4(F2)→v5(F4)→v6(F3)，与执行顺序一致；每个迁移单事务、可从真实旧库升级。
- 类型一致性：`FoodTileStatus`(Rust) ↔ `FoodTileInfo`(TS)；`PushJob/CheckOutcome/settle_pushover`、`PushoverStatus/TestNotifyOutcome` 前后端命名对齐；`ActionTile.foods` 全链路（care→colony→types→ColonyCard）贯通。
- 已知重复造轮子处：QuickLogDialog 与 FeedDialog 样式整段复制（两弹窗职责不同，合并反而耦合食物逻辑，接受）。
