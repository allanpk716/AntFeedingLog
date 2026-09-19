# 界面交互改进第三轮（mock C 系列）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地已与用户逐条确认的 8 项界面/交互改进（见下方决策表），视觉基线为 `mocks/mock-c-*.html` 四份已验收稿。

**Architecture:** 前端 Vue3 手写组件 + Tauri2/WebView2，无 UI 框架（naive-ui 保持不引入）。外壳改为「顶栏固定 + 内容区独立滚动」；自绘 DatePickerPop/DateTimeField 替换全部原生日期控件（共 11 处：打卡/喂食 2、记录页筛选从/到 2、编辑记录 1、新建窝 1、冬眠弹窗 5）；新增 Rust 命令 `colony_month_records` 支撑日历记录标记与重复黄条；`list_logs` 加地点筛选与 location_name。

**Tech Stack:** Vue 3.5 + vitest + @vue/test-utils + happy-dom；Rust (tauri 2, rusqlite, chrono)。

**Spec:** 用户对话定稿（本文件「定稿决策」节即规格摘要）+ `mocks/mock-c-home-cards.html`、`mocks/mock-c-home-rows.html`（未选中方向，仅参考）、`mocks/mock-c-logs.html`、`mocks/mock-c-calendar.html`。首页走**方向一（紧凑卡片）**。

> **rev1（2026-09-19 夜链复审修订）**：消化 codex+pi 双家盲评 19 条已证实问题（对照 `.xcheck/20260919-115517/SUMMARY.md`）——分钟步进定稿为算术进位、标记名字表改全量（colony.actions）、月份加载加过期响应守卫与跨月重同步、防抖重置竞态、弹层水平钳制、卡片菜单关闭路径、shell 真幂等、日期控件计数 6→11 处、4 处测试断言自相矛盾修正、Task 3 提交前全量测试。原稿 `2026-09-19-ui-interaction-round3.md` 不动，**本文件为执行基线**。

## 定稿决策（规格摘要——所有任务以此为准）

| # | 需求 | 定案 |
|---|------|------|
| 1 | 地点筛选 | 记录页加地点下拉，**级联**窝下拉（选地点后窝下拉只列该地点的窝；原选中窝不在列则清空为"全部"） |
| 2 | 右键 | 全局屏蔽 contextmenu；target 为 input/textarea/contenteditable 时放行 |
| 3 | 日历 | **自绘**：素版（纯选日期）+ 带标记版（当前操作橙点、其它操作灰点、悬停 tip）+ 时间快捷键（现在/−10分/＋10分 + 时分步进）；选中即关、弹层不超窗（空间不足向上翻）；替换全部原生 date/datetime-local 输入 |
| 4 | 即选即查 | 下拉/日期变更立即查询；关键词 300ms 防抖；**删除「查询」按钮**，保留「重置」 |
| 5 | 位置 | LogRow 加 `location_name`；表格窝名下方灰色小字显示地点（null 显示「未分组」） |
| 6 | 滚动 | 顶栏固定；内容区独立滚动；消灭常驻滚动条（去 min-height:100vh 叠加与 80px 底衬）；默认窗口 960×680 |
| 7 | 首页 | 方向一紧凑卡片：去开始日期行、操作块单行 chip（2 列）、饲养天数内联、最近记录单行截断、编辑/冬眠按钮收进「⋯」菜单（**v-show 保 DOM，既有测试选择器不破坏**）、「＋新建窝」挪顶栏、去掉页底大按钮 |
| 8 | 重复提醒 | 「该窝+该操作+选中日期」已有记录 → 面板黄条提醒（含编辑记录改日期、含喂食），**不拦提交**；数据源 `colony_month_records` |

## Global Constraints

- 包管理器 `pnpm`；前端测试 `pnpm vitest run <file>`，全量 `pnpm test`；Rust 测试 `cd src-tauri && cargo test <name>`（工作目录在 `src-tauri`）。
- **不新增任何依赖**（naive-ui 已装但禁用）。
- 前端 DTO 字段 snake_case 与 Rust serde 对齐；Tauri 命令参数 JS 侧 camelCase（现状如 `invoke("start_hibernation", { colonyId, startDate })`）。
- 规则 10（停用操作/食物：编辑原引用可保留、新挂拒绝）在所有改动中保持口径。
- 视觉 token 沿用 `mocks/mock-a-light.html` 的 CSS 变量（--bg/--accent 等）。
- 本工作区是 git worktree（`改进界面交互`），禁止 `git stash` 裸操作；提交信息用中文 conventional commits，结尾加：
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- 每个任务结束必须 `pnpm test` 全绿 +（涉及 Rust 时）`cargo test` 全绿再提交。

## File Structure（总览）

- 新建 `src/lib/shell.ts` + 测试：右键屏蔽安装器
- 新建 `src/lib/calendar.ts` + 测试：月格/时间步进纯函数
- 新建 `src/lib/monthview.ts` + 测试：标记构建 + 重复判定 + 提醒文案
- 新建 `src/components/DatePickerPop.vue` + 测试：素版日历弹层（标记层内建）
- 新建 `src/components/DateTimeField.vue` + 测试：日期触发器 + 素版/带标记弹层 + 时间快捷行
- 改 `src/App.vue`、`src/main.ts`、`index.html`、`src-tauri/tauri.conf.json`：外壳
- 改 `src/components/ColonyCard.vue`：紧凑卡片
- 改 `src/components/QuickLogDialog.vue`、`FeedDialog.vue`：DateTimeField + 黄条
- 改 `src-tauri/src/care.rs`、`src-tauri/src/lib.rs`：colony_month_records / list_logs 地点
- 改 `src/types.ts`、`src/lib/loglist.ts` + 测试：DTO 与级联纯函数
- 改 `src/components/LogListPage.vue` + 测试：记录页改版
- 改 `src/components/ColonyFormDialog.vue`、`HibernationDialog.vue`：素版日历
- 改 `src/App.test.ts`：外壳/紧凑卡片/弹窗控件适配
- 改 `README.md`：功能清单与测试数

---

### Task 1: 应用外壳——顶栏固定 + 内容独立滚动 + 右键屏蔽 + 窗口 960×680

**Files:**
- Create: `src/lib/shell.ts`、`src/lib/shell.test.ts`
- Modify: `src/main.ts`、`index.html`、`src/App.vue`、`src-tauri/tauri.conf.json`、`src/components/LogListPage.vue:351`
- Test: `src/lib/shell.test.ts`、`src/App.test.ts`

**Interfaces:**
- Consumes: 无
- Produces: `installContextMenuShield(): void`（main.ts 调用一次）；App.vue 新增 `.page-body` 滚动容器类（后续任务的页面都在其中渲染，无需再改）

- [ ] **Step 1: 写右键屏蔽的失败测试**

`src/lib/shell.test.ts`：

```ts
import { afterEach, describe, expect, it } from "vitest";
import { installContextMenuShield } from "./shell";

/** 在 target 上派发 contextmenu，返回是否被 preventDefault */
function fire(el: Element): boolean {
  const e = new Event("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(e);
  return e.defaultPrevented;
}

describe("installContextMenuShield", () => {
  afterEach(() => {
    // 卸载：shield 用幂等安装，测试间用新元素隔离即可
  });

  it("屏蔽普通元素与文本节点的右键菜单", () => {
    const off = installContextMenuShield();
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(fire(div)).toBe(true);
    div.remove();
    off();
  });

  it("输入框 / 文本域 / 可编辑区内放行（保留右键粘贴）", () => {
    const off = installContextMenuShield();
    const input = document.createElement("input");
    const ta = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    document.body.append(input, ta, editable);
    expect(fire(input)).toBe(false);
    expect(fire(ta)).toBe(false);
    expect(fire(editable)).toBe(false);
    input.remove(); ta.remove(); editable.remove();
    off();
  });

  it("重复安装幂等（同一卸载器、不叠监听）；off 后不再屏蔽，可重装", () => {
    const off1 = installContextMenuShield();
    const off2 = installContextMenuShield();
    expect(off2).toBe(off1); // 复审 #13/#14：同一卸载器，而非各挂各的
    const div = document.createElement("div");
    document.body.appendChild(div);
    const e = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    off1();
    off2(); // 同一函数，重复调用无害
    const e2 = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e2);
    expect(e2.defaultPrevented).toBe(false);
    const off3 = installContextMenuShield(); // 卸载后可重装
    const e3 = new Event("contextmenu", { bubbles: true, cancelable: true });
    div.dispatchEvent(e3);
    expect(e3.defaultPrevented).toBe(true);
    off3();
    div.remove();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/lib/shell.test.ts`
Expected: FAIL（`Cannot find module './shell'`）

- [ ] **Step 3: 实现 shell.ts**

```ts
/**
 * 应用外壳行为（交互改进第三轮 #2）：屏蔽 WebView 默认右键菜单——
 * 桌面应用里浏览器菜单（刷新/检查）露馅；输入类控件放行（右键粘贴有用）。
 * 幂等安装，返回卸载函数（目前仅测试用）。
 */

function isEditable(target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : null;
  if (el === null) return false;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return true;
  return el.isContentEditable;
}

let offShield: (() => void) | null = null;

export function installContextMenuShield(): () => void {
  if (offShield !== null) return offShield; // 复审 #13/#14：真幂等——重复安装返回同一卸载器，不叠监听
  const handler = (e: MouseEvent) => {
    if (isEditable(e.target)) return;
    e.preventDefault();
  };
  document.addEventListener("contextmenu", handler);
  offShield = () => {
    document.removeEventListener("contextmenu", handler);
    offShield = null;
  };
  return offShield;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm vitest run src/lib/shell.test.ts`
Expected: PASS 3 个

- [ ] **Step 5: main.ts 接线 + index.html 高度基线**

`src/main.ts` 在 `createApp(App).mount("#app")` 之前加：

```ts
import { installContextMenuShield } from "./lib/shell";
// 交互第三轮 #2：屏蔽 web 原生右键菜单（输入框内保留）
installContextMenuShield();
```

`index.html` 的 `<head>` 内加：

```html
    <style>
      /* 交互第三轮 #6：外壳占满视口不滚动（滚动移交给 .page-body），顺带清 body 默认 margin */
      html, body, #app { height: 100%; margin: 0; }
    </style>
```

- [ ] **Step 6: App.vue 外壳改版**

`src/App.vue` `<template>` 中，把三个页面（`<main v-if="page === 'home'">`、`<StatsPage/>`、`<LogListPage/>`）包进一个滚动容器：

```html
    <!-- 交互第三轮 #6：顶栏固定，内容区独立滚动 -->
    <div class="page-body">
      <main v-if="page === 'home'" class="container">
        <!-- …原 main 内部不变… -->
      </main>

      <StatsPage v-if="page === 'stats'" />

      <!-- 记录流水（票 08）：任何编辑/删除抛 changed → refresh，首页红绿态即时重算 -->
      <LogListPage v-if="page === 'logs'" @changed="void refresh()" />
    </div>
```

`<style scoped>` 中 `.page` 与 `.container` 改为（替换原 `min-height: 100vh` 块与 `.container` 规则）：

```css
.page {
  /* 视觉基线 mocks/mock-a-light.html */
  --bg: #f6f4f1;
  --card: #ffffff;
  --tile: #faf8f5;
  --text: #2c2822;
  --muted: #8f887d;
  --border: #e8e2d8;
  --border-strong: #d8d1c4;
  --accent: #d97706;
  --accent-deep: #b45309;
  --accent-soft: #fdf1de;
  --ok: #188a4b;
  --ok-soft: #e7f5ec;
  --bad: #d13d3d;
  --bad-soft: #fcebeb;
  --hib: #5b6472;
  --hib-soft: #eef0f3;
  --shadow: 0 1px 2px rgba(60, 50, 30, 0.05), 0 4px 14px rgba(60, 50, 30, 0.06);
  --overlay: rgba(40, 35, 25, 0.35);

  height: 100vh;          /* 交互第三轮 #6：外壳固定，不再整页滚 */
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  font-size: 14px;
  line-height: 1.55;
}

.topbar {
  flex: none;             /* 顶栏固定高度，不参与滚动 */
  background: var(--card);
  border-bottom: 1px solid var(--border);
}

/* 唯一滚动容器：切换页面只有这里滚，顶栏常驻 */
.page-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.container {
  max-width: 1080px;
  margin: 0 auto;
  padding: 0 20px 32px;   /* 原 80px 底衬是常驻滚动条元凶之一 */
}
```

`src/components/LogListPage.vue:351` 同步：`padding: 0 20px 80px;` → `padding: 0 20px 32px;`。

`src-tauri/tauri.conf.json` 窗口尺寸：`"width": 800` → `960`，`"height": 600` → `680`。

- [ ] **Step 7: App.test.ts 补外壳断言 + 全量验证**

`src/App.test.ts` 「顶栏导航」describe 里加一个用例：

```ts
  it("外壳：顶栏之外有独立滚动容器 .page-body（交互第三轮 #6）", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find(".page-body").exists()).toBe(true);
    expect(wrapper.find(".topbar").exists()).toBe(true);
  });
```

Run: `pnpm test`
Expected: 全绿（既有导航/首页用例不依赖被移除的结构）
再跑 `cd src-tauri && cargo test`（本任务无 Rust 改动，确认无意外）。

- [ ] **Step 8: Commit**

```bash
git add src/lib/shell.ts src/lib/shell.test.ts src/main.ts index.html src/App.vue src/App.test.ts src/components/LogListPage.vue src-tauri/tauri.conf.json
git commit -m "feat(shell): 顶栏固定+内容区独立滚动消灭常驻滚动条；屏蔽web右键(输入框放行)；默认窗口960x680

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: lib/calendar.ts 纯函数 + DatePickerPop 素版日历

**Files:**
- Create: `src/lib/calendar.ts`、`src/lib/calendar.test.ts`、`src/components/DatePickerPop.vue`、`src/components/DatePickerPop.test.ts`
- Test: 上述两个 .test.ts

**Interfaces:**
- Consumes: `todayIso()`（`src/lib/dates.ts` 已有）
- Produces（后续任务依赖，签名不得变）:
  - `monthGrid(year: number, month: number): { iso: string | null }[]` — 周一起排，前置 null 填充，仅当月真实日期给 iso
  - `shiftMinutes(value: string, delta: number): string`、`stepTimeField(value: string, field: "h" | "m", delta: number): string` — value 形如 `2026-09-19T20:05`，跨日/进位安全
  - `weekdayShort(iso: string): string` — 返回 "一"…"日"
  - 组件 `DatePickerPop`：props `{ modelValue: string; placeholder?: string; markers?: Record<string, { current: boolean; tip: string }> }`，emits `update:modelValue` / `month`（payload `{ year: number; month: number }`，挂载时与每次翻页后都发）

- [ ] **Step 1: 写 calendar.ts 失败测试**

`src/lib/calendar.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { monthGrid, shiftMinutes, stepTimeField, weekdayShort } from "./calendar";

describe("monthGrid（周一起排）", () => {
  it("2026-09：1 号是周二 → 首格 null，共 1 前置 + 30 天", () => {
    const cells = monthGrid(2026, 9);
    expect(cells).toHaveLength(31);
    expect(cells[0].iso).toBeNull();
    expect(cells[1].iso).toBe("2026-09-01");
    expect(cells[30].iso).toBe("2026-09-30");
  });

  it("2026-08：8-1 是周六 → 5 前置；2024-02（闰）29 天；2023-02 28 天", () => {
    expect(monthGrid(2026, 8).filter((c) => c.iso === null)).toHaveLength(5);
    expect(monthGrid(2024, 2).at(-1)!.iso).toBe("2024-02-29");
    expect(monthGrid(2023, 2).at(-1)!.iso).toBe("2023-02-28");
  });

  it("月份越界抛错（组件层负责钳制）", () => {
    expect(() => monthGrid(2026, 0)).toThrow();
    expect(() => monthGrid(2026, 13)).toThrow();
  });
});

describe("weekdayShort", () => {
  it("2026-09-19 是周六", () => {
    expect(weekdayShort("2026-09-19")).toBe("六");
  });
});

describe("时间步进（value = YYYY-MM-DDTHH:mm）", () => {
  it("shiftMinutes：普通加减、借位跨日回绕", () => {
    expect(shiftMinutes("2026-09-19T20:05", -10)).toBe("2026-09-19T19:55");
    expect(shiftMinutes("2026-09-19T00:05", -10)).toBe("2026-09-18T23:55");
    expect(shiftMinutes("2026-09-19T23:55", 10)).toBe("2026-09-20T00:05");
  });

  it("stepTimeField：分钟算术进位（59+1→下一小时 00），小时 23↔0 回绕，日期不动", () => {
    expect(stepTimeField("2026-09-19T20:05", "h", 1)).toBe("2026-09-19T21:05");
    expect(stepTimeField("2026-09-19T23:05", "h", 1)).toBe("2026-09-19T00:05");
    expect(stepTimeField("2026-09-19T20:59", "m", 1)).toBe("2026-09-19T21:00");
    expect(stepTimeField("2026-09-19T20:00", "m", -1)).toBe("2026-09-19T19:59");
    // 复审定稿语义：步进只调时刻、不跨日（跨日走 ±10分/现在）；00:00 −1分 回到当天 23:59
    expect(stepTimeField("2026-09-19T00:00", "m", -1)).toBe("2026-09-19T23:59");
  });

  it("非法输入原样返回（脏值兜底，不抛错）", () => {
    expect(shiftMinutes("garbage", -10)).toBe("garbage");
    expect(stepTimeField("", "h", 1)).toBe("");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/lib/calendar.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 calendar.ts**

```ts
/**
 * 自绘日历纯逻辑（交互第三轮 #3）：月格构建 + datetime-local 形态字符串的时间步进。
 * 组件只管渲染与弹层，格子/进位规则全部在这里，口径与 mocks/mock-c-calendar.html 一致。
 */

/** 周一起排的月格：前置 null 填充（周列头由组件渲染），当月每天给 ISO 日期 */
export function monthGrid(year: number, month: number): { iso: string | null }[] {
  if (month < 1 || month > 12) {
    throw new Error(`月份应在 1–12：${year}-${month}`);
  }
  const lead = (new Date(year, month - 1, 1).getDay() + 6) % 7; // 周一=0
  const days = new Date(year, month, 0).getDate();
  const cells: { iso: string | null }[] = Array.from({ length: lead }, () => ({ iso: null }));
  for (let d = 1; d <= days; d++) {
    cells.push({ iso: `${year}-${String(month).padStart(2, "0")}-${String(d).padStart(2, "0")}` });
  }
  return cells;
}

/** ISO 日期 → 一/二/…/日（标题展示用） */
export function weekdayShort(iso: string): string {
  return "一二三四五六日"[(new Date(`${iso}T00:00`).getDay() + 6) % 7];
}

/** "YYYY-MM-DDTHH:mm" → 总分钟数；解析失败 NaN */
function toMinutes(value: string): number {
  const m = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/);
  if (!m) return Number.NaN;
  const [, y, mo, d, h, mi] = m;
  return Date.UTC(+y, +mo - 1, +d, +h, +mi) / 60000;
}

function fromMinutes(total: number): string {
  const dt = new Date(total * 60000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${dt.getUTCFullYear()}-${p(dt.getUTCMonth() + 1)}-${p(dt.getUTCDate())}T${p(dt.getUTCHours())}:${p(dt.getUTCMinutes())}`;
}

/** 整体平移分钟（现在/±10分快捷键）；跨日安全；脏值原样返回 */
export function shiftMinutes(value: string, delta: number): string {
  const t = toMinutes(value);
  return Number.isNaN(t) ? value : fromMinutes(t + delta);
}

/** 单字段步进（时/分 ‹›）——复审定稿语义：分钟算术进位（59+1→下一小时 00）、
 * 小时 23↔0 回绕；步进不跨日（跨日走 ±10分/现在）；脏值原样返回。 */
export function stepTimeField(value: string, field: "h" | "m", delta: number): string {
  const m = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/);
  if (!m) return value;
  const [, y, mo, d, h, mi] = m;
  const total =
    field === "h"
      ? ((((+h + delta) % 24) + 24) % 24) * 60 + +mi // 小时回绕，分钟不动
      : (+h * 60 + +mi + delta + 1440) % 1440; // 分钟算术进位可跨小时，日期不动
  const p = (n: number) => String(n).padStart(2, "0");
  return `${y}-${mo}-${d}T${p(Math.floor(total / 60))}:${p(total % 60)}`;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm vitest run src/lib/calendar.test.ts`
Expected: PASS

- [ ] **Step 5: 写 DatePickerPop 失败测试**

`src/components/DatePickerPop.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import DatePickerPop from "./DatePickerPop.vue";

const markers = {
  "2026-09-12": { current: true, tip: "清理×1（19:40）· 保湿×1" },
  "2026-09-17": { current: false, tip: "喂食×1（种子）· 保湿×1" },
};

function mountPop(modelValue = "") {
  return mount(DatePickerPop, { props: { modelValue, markers, placeholder: "开始日期" } });
}

function open(w: ReturnType<typeof mountPop>) {
  return w.find(".dp-trigger").trigger("click");
}

describe("DatePickerPop（素版 + 标记层）", () => {
  it("初始关闭；点触发器展开并渲染 2026-09 月格（默认看选中/今天所在月）", async () => {
    const w = mountPop("2026-09-05");
    expect(w.find(".dp-pop").exists()).toBe(false);
    await open(w);
    expect(w.find(".dp-pop").exists()).toBe(true);
    expect(w.find(".dp-title").text()).toBe("2026年9月");
    expect(w.findAll(".dp-day.dim").length).toBe(1); // 2026-09-01 是周二 → 1 个前置格（复审 #5：dim 同带 .dp-day 类）
    expect(w.findAll(".dp-day:not(.dim)").length).toBe(30);
  });

  it("点某天 → 发 update:modelValue 并立即关闭（选中即关）", async () => {
    const w = mountPop("2026-09-05");
    await open(w);
    await w.findAll(".dp-day").find((d) => d.text() === "12")!.trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-12"]);
    expect(w.find(".dp-pop").exists()).toBe(false);
  });

  it("标记：current 日渲染橙点 .dot.cur，其余渲染灰点；悬停 title = tip", async () => {
    const w = mountPop("2026-09-19");
    await open(w);
    const days = w.findAll(".dp-day");
    const d12 = days.find((d) => d.text() === "12")!;
    const d17 = days.find((d) => d.text() === "17")!;
    expect(d12.find(".dot.cur").exists()).toBe(true);
    expect(d12.attributes("title")).toContain("清理×1");
    expect(d17.find(".dot.cur").exists()).toBe(false);
    expect(d17.find(".dot").exists()).toBe(true);
    expect(d17.attributes("title")).toContain("喂食×1");
  });

  it("翻月与「今天」：month 事件随翻页发出；空值挂载从今天所在月开始", async () => {
    const w = mountPop("");
    await open(w);
    // 空值默认今天（测试环境日期不定，只断言能开、有标题）
    expect(w.find(".dp-title").text()).toMatch(/^\d{4}年\d{1,2}月$/);
    await w.find(".dp-prev").trigger("click");
    const evts = w.emitted("month")!;
    expect(evts.at(-1)![0]).toHaveProperty("year");
    await w.find(".dp-today").trigger("click");
    expect(w.find(".dp-title").text()).toMatch(/^\d{4}年\d{1,2}月$/);
  });

  it("无 markers 也能用（素版场景）", async () => {
    const w = mount(DatePickerPop, { props: { modelValue: "2026-09-19" } });
    await open(w);
    expect(w.find(".dp-day .dot").exists()).toBe(false);
  });
});
```

- [ ] **Step 6: 跑测试确认失败**

Run: `pnpm vitest run src/components/DatePickerPop.test.ts`
Expected: FAIL（组件不存在）

- [ ] **Step 7: 实现 DatePickerPop.vue**

```vue
<script setup lang="ts">
/**
 * 自绘日历弹层（交互第三轮 #3，视觉基线 mocks/mock-c-calendar.html）：
 * 选中即关、翻月/今天、Esc 与点外部关闭、向下空间不足自动向上弹（不超窗）。
 * markers 传入即带记录标记层（当前操作橙点/其它灰点/悬停 tip），不传即素版。
 * 纯日期版；datetime 场景由 DateTimeField 组合本组件 + 时间快捷行。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { monthGrid } from "../lib/calendar";
import { todayIso } from "../lib/dates";

const props = withDefaults(
  defineProps<{
    modelValue: string;
    placeholder?: string;
    markers?: Record<string, { current: boolean; tip: string }>;
  }>(),
  { placeholder: "选择日期", markers: () => ({}) },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
  month: [view: { year: number; month: number }];
}>();

/** 当前展示月（初始 = 选中值或今天所在月） */
const today = todayIso();
const initial = props.modelValue !== "" ? props.modelValue : today;
const view = ref({ year: +initial.slice(0, 4), month: +initial.slice(5, 7) });

const open = ref(false);
const flipUp = ref(false);
const triggerRef = ref<HTMLElement | null>(null);

const cells = computed(() => monthGrid(view.value.year, view.value.month));
const label = computed(() => `${view.value.year}年${view.value.month}月`);
const display = computed(() =>
  props.modelValue === "" ? props.placeholder : props.modelValue,
);

/** 展示月变化（含翻页/今天跳转）都通知父级，供其拉该月标记数据 */
function notifyMonth() {
  emit("month", { ...view.value });
}
onMounted(notifyMonth);

function nav(delta: number) {
  let m = view.value.month + delta;
  let y = view.value.year;
  if (m < 1) { m = 12; y -= 1; }
  if (m > 12) { m = 1; y += 1; }
  view.value = { year: y, month: m };
  notifyMonth();
}

function goToday() {
  view.value = { year: +today.slice(0, 4), month: +today.slice(5, 7) };
  notifyMonth();
}

const popShift = ref(0);
function toggle() {
  if (open.value) { open.value = false; return; }
  const rect = triggerRef.value?.getBoundingClientRect();
  if (rect !== undefined) {
    // 弹层翻转判定：触发器下方空间 < 320px 就向上弹（弹层约 300px 高）
    flipUp.value = window.innerHeight - rect.bottom < 320 && rect.top > 320;
    // 复审 #11：水平钳制——右缘放不下 252px 弹层时整体左移，保证不出窗
    popShift.value = Math.min(0, window.innerWidth - rect.left - 252 - 12);
  }
  open.value = true;
}

function pick(iso: string) {
  open.value = false;                 // 选中即关（mock C4 定稿行为）
  emit("update:modelValue", iso);
}

function onDocClick(e: MouseEvent) {
  if (!open.value) return;
  const root = triggerRef.value?.parentElement;
  if (root !== null && root !== undefined && e.target instanceof Node && root.contains(e.target)) return;
  open.value = false;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") open.value = false;
}
onMounted(() => {
  document.addEventListener("click", onDocClick);
  document.addEventListener("keydown", onKey);
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
  document.removeEventListener("keydown", onKey);
});

watch(
  () => props.modelValue,
  (v) => {
    if (v !== "") view.value = { year: +v.slice(0, 4), month: +v.slice(5, 7) };
  },
);
</script>

<template>
  <span class="dp">
    <button ref="triggerRef" class="dp-trigger" :class="{ empty: modelValue === '' }" type="button" @click="toggle">
      {{ display }}<span class="dp-caret">▾</span>
    </button>
    <div v-if="open" class="dp-pop" :class="{ up: flipUp }" :style="{ left: popShift + 'px' }">
      <div class="dp-head">
        <button class="dp-nav dp-prev" type="button" @click="nav(-1)">‹</button>
        <span class="dp-title">{{ label }}</span>
        <button class="dp-nav dp-next" type="button" @click="nav(1)">›</button>
        <button class="dp-today" type="button" @click="goToday">今天</button>
      </div>
      <div class="dp-grid">
        <span v-for="w in '一二三四五六日'" :key="w" class="dp-wd">{{ w }}</span>
        <template v-for="(c, i) in cells" :key="i">
          <span v-if="c.iso === null" class="dp-day dim"></span>
          <button
            v-else
            class="dp-day"
            :class="{ sel: c.iso === modelValue, tod: c.iso === today }"
            :title="markers[c.iso]?.tip"
            type="button"
            @click="pick(c.iso)"
          >
            {{ +c.iso.slice(8, 10) }}
            <span v-if="markers[c.iso]" class="dp-dots">
              <span v-if="markers[c.iso]!.current" class="dot cur"></span>
              <span v-else class="dot"></span>
            </span>
          </button>
        </template>
      </div>
      <div v-if="Object.keys(markers).length > 0" class="dp-legend">
        <span class="k"><span class="dot cur"></span>当天已有当前操作</span>
        <span class="k"><span class="dot"></span>其它操作（悬停看明细）</span>
      </div>
      <div class="dp-foot">选中即关闭 · Esc / 点外部关闭</div>
    </div>
  </span>
</template>

<style scoped>
.dp { position: relative; display: inline-block; }
.dp-trigger {
  padding: 6px 10px; border: 1px solid var(--border-strong, #d8d1c4); border-radius: 8px;
  font: inherit; font-size: 13px; background: var(--card, #fff); color: var(--text, #2c2822);
  cursor: pointer; min-width: 104px; text-align: left;
}
.dp-trigger:hover { border-color: var(--accent, #d97706); }
.dp-trigger.empty { color: var(--muted, #8f887d); }
.dp-caret { float: right; color: var(--muted, #8f887d); margin-left: 6px; }

.dp-pop {
  position: absolute; top: calc(100% + 6px); left: 0; z-index: 60;
  width: 252px; background: var(--card, #fff); border: 1px solid var(--border-strong, #d8d1c4);
  border-radius: 12px; box-shadow: 0 10px 40px rgba(60, 50, 30, 0.2); padding: 10px 12px;
}
.dp-pop.up { top: auto; bottom: calc(100% + 6px); }
.dp-head { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; }
.dp-title { font-size: 13px; font-weight: 700; flex: 1; text-align: center; }
.dp-nav, .dp-today {
  border: 1px solid var(--border, #e8e2d8); background: var(--tile, #faf8f5); border-radius: 7px;
  cursor: pointer; font: inherit; font-size: 12px; padding: 1px 8px; color: var(--text, #2c2822);
}
.dp-today { border-radius: 999px; font-size: 11px; color: var(--muted, #8f887d); }
.dp-nav:hover, .dp-today:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dp-grid { display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; text-align: center; }
.dp-wd { font-size: 10px; color: var(--muted, #8f887d); padding: 2px 0; }
.dp-day {
  position: relative; border: none; background: transparent; font: inherit; font-size: 12px;
  color: var(--text, #2c2822); border-radius: 8px; padding: 3px 0 9px; cursor: pointer; min-width: 28px;
}
.dp-day:hover { background: var(--accent-soft, #fdf1de); }
.dp-day.dim { cursor: default; }
.dp-day.dim:hover { background: transparent; }
.dp-day.sel { background: var(--accent, #d97706); color: #fff; font-weight: 700; }
.dp-day.tod:not(.sel) { box-shadow: inset 0 0 0 1.5px var(--accent, #d97706); color: var(--accent-deep, #b45309); font-weight: 600; }
.dp-dots { position: absolute; left: 0; right: 0; bottom: 2px; display: flex; justify-content: center; height: 5px; }
.dot { width: 5px; height: 5px; border-radius: 50%; background: #c9c1b2; }
.dot.cur { width: 7px; height: 7px; background: var(--accent, #d97706); box-shadow: 0 0 0 2px var(--accent-soft, #fdf1de); }
.dp-day.sel .dot { background: rgba(255, 255, 255, 0.75); }
.dp-day.sel .dot.cur { background: #fff; box-shadow: none; }
.dp-legend {
  margin-top: 7px; padding-top: 7px; border-top: 1px dashed var(--border, #e8e2d8);
  font-size: 10px; color: var(--muted, #8f887d); display: flex; gap: 10px; justify-content: center; flex-wrap: wrap;
}
.dp-legend .k { display: flex; align-items: center; gap: 4px; }
.dp-legend .dot { position: static; }
.dp-foot { margin-top: 4px; font-size: 10px; color: var(--muted, #8f887d); text-align: center; }
</style>
```

- [ ] **Step 8: 跑测试确认通过 + 全量**

Run: `pnpm vitest run src/components/DatePickerPop.test.ts src/lib/calendar.test.ts`
Expected: PASS
Run: `pnpm test` → 全绿。

- [ ] **Step 9: Commit**

```bash
git add src/lib/calendar.ts src/lib/calendar.test.ts src/components/DatePickerPop.vue src/components/DatePickerPop.test.ts
git commit -m "feat(ui): 自绘日历DatePickerPop素版+标记层；月格/时间步进纯函数calendar.ts

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: DateTimeField——日期触发器 + 带标记弹层 + 时间快捷行

**Files:**
- Create: `src/components/DateTimeField.vue`、`src/components/DateTimeField.test.ts`
- Test: `src/components/DateTimeField.test.ts`

**Interfaces:**
- Consumes: `DatePickerPop`（Task 2）、`shiftMinutes` / `stepTimeField` / `weekdayShort`（Task 2）、`nowLocalDateTime()`（`src/lib/care.ts` 已有）
- Produces: 组件 `DateTimeField`：props `{ modelValue: string; markers?: Record<string, { current: boolean; tip: string }> }`，emits `update:modelValue` / `month`（转发 DatePickerPop）；modelValue 形如 `2026-09-19T20:05`（与原 datetime-local 完全同形态，**载荷与校验零改动**）

- [ ] **Step 1: 写失败测试**

`src/components/DateTimeField.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import DateTimeField from "./DateTimeField.vue";

function mountField(value = "2026-09-19T20:05") {
  return mount(DateTimeField, { props: { modelValue: value } });
}

describe("DateTimeField", () => {
  it("触发器显示日期+周几；弹层选别的日期 → 发合成后的 datetime", async () => {
    const w = mountField("2026-09-19T20:05");
    expect(w.find(".dp-trigger").text()).toContain("2026-09-19");
    await w.find(".dp-trigger").trigger("click");
    await w.findAll(".dp-day").find((d) => d.text() === "16")!.trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["2026-09-16T20:05"]);
  });

  it("时间快捷：−10分 / ＋10分 / 现在（现在=当前时刻形态）", async () => {
    const w = mountField("2026-09-19T20:05");
    await w.find(".dtf-m10").trigger("click");
    expect(w.emitted("update:modelValue")!.at(-1)).toEqual(["2026-09-19T19:55"]);
    await w.find(".dtf-p10").trigger("click");
    expect(w.emitted("update:modelValue")!.at(-1)).toEqual(["2026-09-19T20:05"]);
    await w.find(".dtf-now").trigger("click");
    const v = w.emitted("update:modelValue")!.at(-1)![0] as string;
    expect(v).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
  });

  it("时分步进按钮：h+/m- 各自回绕；时间部分显示正确", async () => {
    const w = mountField("2026-09-19T23:05");
    const nums = w.findAll(".dtf-time .num");
    expect(nums.map((n) => n.text())).toEqual(["23", "05"]); // 复审 #6：‹› 按钮字符夹在中间，不能 toContain("23:05")
    await w.find("[data-step='h+']").trigger("click");
    expect(w.emitted("update:modelValue")!.at(-1)).toEqual(["2026-09-19T00:05"]);
    await w.find("[data-step='m-']").trigger("click");
    expect(w.emitted("update:modelValue")!.at(-1)).toEqual(["2026-09-19T00:04"]);
  });

  it("month 事件从 DatePickerPop 转发", async () => {
    const w = mountField("2026-09-19T20:05");
    await w.find(".dp-trigger").trigger("click");
    await w.find(".dp-prev").trigger("click");
    const evts = w.emitted("month")!;
    expect(evts.length).toBeGreaterThanOrEqual(2); // 挂载初发 + 翻页
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/components/DateTimeField.test.ts`
Expected: FAIL（组件不存在）

- [ ] **Step 3: 实现 DateTimeField.vue**

```vue
<script setup lang="ts">
/**
 * 日期+时间选择（交互第三轮 #3）：DatePickerPop 管日期，时间行给
 * 「现在/−10分/＋10分」快捷键与时分步进（mocks/mock-c-calendar.html ①区）。
 * modelValue 与原 input[type=datetime-local] 同形态（YYYY-MM-DDTHH:mm），
 * 替换点对弹窗载荷/校验零侵入。
 */
import { computed } from "vue";
import DatePickerPop from "./DatePickerPop.vue";
import { shiftMinutes, stepTimeField, weekdayShort } from "../lib/calendar";
import { nowLocalDateTime } from "../lib/care";

const props = withDefaults(
  defineProps<{ modelValue: string; markers?: Record<string, { current: boolean; tip: string }> }>(),
  { markers: () => ({}) },
);
const emit = defineEmits<{ "update:modelValue": [value: string]; month: [view: { year: number; month: number }] }>();

const datePart = computed(() => props.modelValue.slice(0, 10));
const timePart = computed(() => props.modelValue.slice(11, 16));

function set(value: string) {
  emit("update:modelValue", value);
}
function pickDate(iso: string) {
  set(`${iso}T${timePart.value}`);
}
function shift(delta: number) {
  set(shiftMinutes(props.modelValue, delta));
}
function step(field: "h" | "m", delta: number) {
  set(stepTimeField(props.modelValue, field, delta));
}
</script>

<template>
  <div class="dtf">
    <DatePickerPop
      :model-value="datePart"
      :markers="markers"
      placeholder="选择日期"
      @update:model-value="pickDate"
      @month="(v) => emit('month', v)"
    />
    <div class="dtf-time-row">
      <button class="dtf-chip dtf-now" type="button" @click="set(nowLocalDateTime())">现在</button>
      <button class="dtf-chip dtf-m10" type="button" @click="shift(-10)">−10分</button>
      <button class="dtf-chip dtf-p10" type="button" @click="shift(10)">＋10分</button>
      <span class="dtf-time">
        <span class="seg">
          <button data-step="h-" type="button" @click="step('h', -1)">‹</button>
          <span class="num">{{ timePart.slice(0, 2) }}</span>
          <button data-step="h+" type="button" @click="step('h', 1)">›</button>
        </span>
        <span class="sep">:</span>
        <span class="seg">
          <button data-step="m-" type="button" @click="step('m', -1)">‹</button>
          <span class="num">{{ timePart.slice(3, 5) }}</span>
          <button data-step="m+" type="button" @click="step('m', 1)">›</button>
        </span>
      </span>
      <span v-if="datePart !== ''" class="dtf-wd">周{{ weekdayShort(datePart) }}</span>
    </div>
  </div>
</template>

<style scoped>
.dtf { display: flex; flex-direction: column; gap: 8px; }
.dtf-time-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.dtf-chip {
  border: 1px solid var(--border-strong, #d8d1c4); background: var(--tile, #faf8f5);
  border-radius: 999px; padding: 3px 11px; font: inherit; font-size: 12px;
  color: var(--text, #2c2822); cursor: pointer;
}
.dtf-chip:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dtf-time { display: flex; align-items: center; gap: 3px; margin-left: 4px; font-variant-numeric: tabular-nums; }
.dtf-time .seg { display: flex; align-items: center; gap: 2px; }
.dtf-time .num { font-size: 15px; font-weight: 700; min-width: 26px; text-align: center; }
.dtf-time .sep { font-weight: 700; }
.dtf-time button {
  border: 1px solid var(--border, #e8e2d8); background: var(--tile, #faf8f5); border-radius: 6px;
  cursor: pointer; font: inherit; font-size: 10px; padding: 1px 5px; color: var(--muted, #8f887d); line-height: 1.3;
}
.dtf-time button:hover { border-color: var(--accent, #d97706); color: var(--accent-deep, #b45309); }
.dtf-wd { font-size: 12px; color: var(--muted, #8f887d); }
</style>
```

（触发器即 DatePickerPop 的 `.dp-trigger`（Task 2 已定义），本组件不另设触发器类名，测试直接使用该选择器。）

- [ ] **Step 4: 跑测试确认通过（含全量）**

Run: `pnpm vitest run src/components/DateTimeField.test.ts` → PASS；再跑 `pnpm test` 全量（全局约束：每任务全绿才提交——复审 #19）
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/components/DateTimeField.vue src/components/DateTimeField.test.ts
git commit -m "feat(ui): DateTimeField日期+时间组合控件（现在/±10分快捷+时分步进，转发月事件）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Rust——colony_month_records 命令（标记 + 黄条数据源）

**Files:**
- Modify: `src-tauri/src/care.rs`（新 struct + 函数 + 测试）、`src-tauri/src/lib.rs`（命令注册）、`src/types.ts`
- Test: `src-tauri/src/care.rs` 内 `#[cfg(test)] mod tests` 新增用例

**Interfaces:**
- Consumes: `mem_conn()` 等既有测试 helper（care.rs tests 模块）
- Produces:
  - Rust `care::MonthDayRecords { day: i64, action_id: i64, count: i64, last_time: String }`（serde 默认 snake_case）
  - `care::colony_month_records(conn: &Connection, colony_id: i64, year: i64, month: i64) -> Result<Vec<MonthDayRecords>, String>`
  - Tauri 命令 `colony_month_records`，前端调用 `invoke("colony_month_records", { colonyId, year, month })`
  - TS `MonthDayRecords` 接口

- [ ] **Step 1: 写失败测试（追加到 care.rs tests 模块末尾）**

```rust
    // ── 按窝按月记录摘要（交互第三轮 #8：日历标记 + 重复黄条数据源）──

    fn month_rows(conn: &Connection, colony_id: i64, year: i64, month: i64) -> Vec<MonthDayRecords> {
        colony_month_records(conn, colony_id, year, month).expect("按月查询失败")
    }

    #[test]
    fn colony_month_records_groups_by_day_and_action_with_last_time() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        log(&conn, c, "垃圾清理", "2026-09-12 19:40:00");
        log(&conn, c, "垃圾清理", "2026-09-12 08:30:00"); // 同日同操作第二条
        log(&conn, c, "巢穴保湿", "2026-09-12 21:00:00");
        log(&conn, c, "喂食", "2026-09-17 20:00:00");

        let rows = month_rows(&conn, c, 2026, 9);
        assert_eq!(rows.len(), 3, "3 个 (day, action) 组");
        let trash = rows.iter().find(|r| r.action_id == action_id(&conn, "垃圾清理")).unwrap();
        assert_eq!(trash.day, 12);
        assert_eq!(trash.count, 2);
        assert_eq!(trash.last_time, "2026-09-12 19:40:00"); // 最近一条的时刻
    }

    #[test]
    fn colony_month_records_excludes_other_months_and_colonies() {
        let conn = mem_conn();
        let c1 = colony(&conn, "大头一号");
        let c2 = colony(&conn, "大头二号");
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c1, "喂食", "2026-08-31 23:59:59"); // 上月
        log(&conn, c1, "喂食", "2026-10-01 00:00:00"); // 次月
        log(&conn, c2, "喂食", "2026-09-18 09:00:00"); // 别窝

        let rows = month_rows(&conn, c1, 2026, 9);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].day, 17);
    }

    #[test]
    fn colony_month_records_rejects_bad_month() {
        let conn = mem_conn();
        let c = colony(&conn, "大头一号");
        assert!(colony_month_records(&conn, c, 2026, 0).is_err());
        assert!(colony_month_records(&conn, c, 2026, 13).is_err());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test colony_month_records`
Expected: FAIL（`cannot find function`）

- [ ] **Step 3: 实现（care.rs，放在 list_logs 之后）**

```rust
/// 按窝按月的记录摘要行（交互第三轮 #8）：日历标记与重复提醒的数据源。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonthDayRecords {
    pub day: i64,
    pub action_id: i64,
    pub count: i64,
    /// 该 (日, 操作) 最近一条的发生时刻（"YYYY-MM-DD HH:MM:SS"）
    pub last_time: String,
}

/// 某窝某月每天的逐操作计数（日历橙点/灰点、黄条「已有 N 条（HH:MM）」用）。
/// month 取 1–12；occurred_at 落在 [当月1日, 次月1日) 字典序区间内（库内格式定长，字典序即时间序）。
pub fn colony_month_records(
    conn: &Connection,
    colony_id: i64,
    year: i64,
    month: i64,
) -> Result<Vec<MonthDayRecords>, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("月份应在 1–12：{year}-{month}"));
    }
    let (next_y, next_m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let range_start = format!("{year:04}-{month:02}-01 00:00:00");
    let range_end = format!("{next_y:04}-{next_m:02}-01 00:00:00");

    let mut stmt = conn
        .prepare(
            "SELECT CAST(substr(l.occurred_at, 9, 2) AS INTEGER), l.action_id, COUNT(*), MAX(l.occurred_at)
             FROM care_log l
             WHERE l.colony_id = ?1 AND l.occurred_at >= ?2 AND l.occurred_at < ?3
             GROUP BY substr(l.occurred_at, 9, 2), l.action_id
             ORDER BY substr(l.occurred_at, 9, 2), l.action_id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![colony_id, range_start, range_end], |row| {
            Ok(MonthDayRecords {
                day: row.get(0)?,
                action_id: row.get(1)?,
                count: row.get(2)?,
                last_time: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}
```

`src-tauri/src/lib.rs`：在 `list_logs` 命令之后加（并加入 `generate_handler![...]` 列表，紧挨 `list_logs,`）：

```rust
#[tauri::command]
fn colony_month_records(
    state: tauri::State<'_, DbState>,
    colony_id: i64,
    year: i64,
    month: i64,
) -> Result<Vec<care::MonthDayRecords>, String> {
    with_conn(state, |conn| care::colony_month_records(conn, colony_id, year, month))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test colony_month_records && cargo test`
Expected: PASS（新 3 个 + 既有全绿）

- [ ] **Step 5: types.ts 对齐 + Commit**

`src/types.ts` 末尾追加：

```ts
/** 按窝按月记录摘要行（colony_month_records；交互第三轮：日历标记 + 重复提醒数据源） */
export interface MonthDayRecords {
  day: number;
  action_id: number;
  count: number;
  /** 该 (日, 操作) 最近一条发生时刻（"YYYY-MM-DD HH:MM:SS"） */
  last_time: string;
}
```

```bash
git add src-tauri/src/care.rs src-tauri/src/lib.rs src/types.ts
git commit -m "feat(backend): colony_month_records按窝按月逐操作计数——日历标记与重复提醒的数据源

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: monthview 纯函数 + QuickLogDialog / FeedDialog 接入标记日历与黄条

**Files:**
- Create: `src/lib/monthview.ts`、`src/lib/monthview.test.ts`
- Modify: `src/components/QuickLogDialog.vue`、`src/components/FeedDialog.vue`、`src/components/QuickLogDialog.test.ts`、`src/App.test.ts`
- Test: 上述两个 .test.ts

**Interfaces:**
- Consumes: `MonthDayRecords`（Task 4）、`DateTimeField`（Task 3）、`CareActionItem`（types）
- Produces:
  - `buildMarkers(rows: MonthDayRecords[], currentActionId: number, actionNames: Map<number, string>): Record<string, { current: boolean; tip: string }>` — key 为 ISO 日期（由 rows + year/month 拼装）
  - `duplicateInfo(rows: MonthDayRecords[], year: number, month: number, dateIso: string, actionId: number): { count: number; lastTime: string } | null` — lastTime 已裁成 "HH:MM"
  - `dupWarningText(dup: { count: number; lastTime: string } | null, dateIso: string, todayIsoStr: string, actionName: string): string` — "今天已有 1 条清理记录（14:32），请确认不是重复操作。" / "9月12日已有 …"

- [ ] **Step 1: 写 monthview.ts 失败测试**

`src/lib/monthview.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { buildMarkers, dupWarningText, duplicateInfo } from "./monthview";
import type { MonthDayRecords } from "../types";

const rows: MonthDayRecords[] = [
  { day: 12, action_id: 4, count: 1, last_time: "2026-09-12 19:40:00" },
  { day: 12, action_id: 3, count: 1, last_time: "2026-09-12 21:00:00" },
  { day: 17, action_id: 1, count: 2, last_time: "2026-09-17 20:00:00" },
  { day: 19, action_id: 4, count: 1, last_time: "2026-09-19 14:32:00" },
];
const names = new Map<number, string>([[1, "喂食"], [3, "巢穴保湿"], [4, "垃圾清理"]]);

describe("buildMarkers", () => {
  it("当前操作日 current=true，tip 汇总当天全部操作", () => {
    const m = buildMarkers(rows, 4, names);
    expect(m["2026-09-12"]).toEqual({ current: true, tip: "垃圾清理×1 · 巢穴保湿×1" });
    expect(m["2026-09-17"]).toEqual({ current: false, tip: "喂食×2" });
    expect(m["2026-09-19"]!.current).toBe(true);
    expect(m["2026-09-01"]).toBeUndefined();
  });
});

describe("duplicateInfo", () => {
  it("选中日 + 同操作 → count/最近时刻（HH:MM）；否则 null", () => {
    expect(duplicateInfo(rows, 2026, 9, "2026-09-19", 4)).toEqual({ count: 1, lastTime: "14:32" });
    expect(duplicateInfo(rows, 2026, 9, "2026-09-17", 4)).toBeNull(); // 有其它操作但无当前操作
    expect(duplicateInfo(rows, 2026, 9, "2026-09-01", 4)).toBeNull(); // 无任何记录
  });
  it("非本月日期直接 null（防串月）", () => {
    expect(duplicateInfo(rows, 2026, 9, "2026-08-19", 4)).toBeNull();
  });
});

describe("dupWarningText", () => {
  it("今天与其它日期的文案形态", () => {
    expect(dupWarningText({ count: 1, lastTime: "14:32" }, "2026-09-19", "2026-09-19", "清理"))
      .toBe("今天已有 1 条清理记录（14:32），请确认不是重复操作。");
    expect(dupWarningText({ count: 2, lastTime: "09:05" }, "2026-09-12", "2026-09-19", "喂食"))
      .toBe("9月12日已有 2 条喂食记录（最近 09:05），请确认不是重复操作。");
    expect(dupWarningText(null, "2026-09-01", "2026-09-19", "清理")).toBe("");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/lib/monthview.test.ts` → FAIL

- [ ] **Step 3: 实现 monthview.ts**

```ts
/**
 * 打卡日历的记录标记与重复提醒纯逻辑（交互第三轮 #3/#8）。
 * 数据源 colony_month_records（一次一窝一月）；口径与 mocks/mock-c-calendar.html 一致：
 * 橙点=当天已有「正在记录的这项操作」，灰点=当天有其它操作，悬停 tip 列明细。
 */
import type { MonthDayRecords } from "../types";

const iso = (year: number, month: number, day: number) =>
  `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;

/** rows → 日历标记表：按日聚合，tip = "操作A×n · 操作B×m"（actionNames 缺名时跳过该行） */
export function buildMarkers(
  rows: MonthDayRecords[],
  currentActionId: number,
  actionNames: Map<number, string>,
): Record<string, { current: boolean; tip: string }> {
  const byDay = new Map<number, MonthDayRecords[]>();
  for (const r of rows) {
    const list = byDay.get(r.day) ?? [];
    list.push(r);
    byDay.set(r.day, list);
  }
  const out: Record<string, { current: boolean; tip: string }> = {};
  for (const [day, list] of byDay) {
    const parts: string[] = [];
    let current = false;
    // 明细按 rows 到达序输出（Rust 端 ORDER BY day, action_id 已定序；复审 #4：不做二次排序，与测试口径一致）
    for (const r of list) {
      const name = actionNames.get(r.action_id);
      if (name === undefined) continue;
      parts.push(`${name}×${r.count}`);
      if (r.action_id === currentActionId) current = true;
    }
    if (parts.length > 0) out[dayKey(list[0]!, day)] = { current, tip: parts.join(" · ") };
  }
  return out;
}

/** rows 里挑出 ISO 日期的 key 用的年份月份（rows 行自带 last_time 可取年月） */
function dayKey(sample: MonthDayRecords, day: number): string {
  const y = +sample.last_time.slice(0, 4);
  const m = +sample.last_time.slice(5, 7);
  return iso(y, m, day);
}

/** 选中日期已有同操作记录 → {count, 最近 HH:MM}；否则 null（跨月/无记录都 null） */
export function duplicateInfo(
  rows: MonthDayRecords[],
  year: number,
  month: number,
  dateIso: string,
  actionId: number,
): { count: number; lastTime: string } | null {
  if (!dateIso.startsWith(`${year}-${String(month).padStart(2, "0")}-`)) return null;
  const day = +dateIso.slice(8, 10);
  const hit = rows.find((r) => r.day === day && r.action_id === actionId);
  return hit === undefined ? null : { count: hit.count, lastTime: hit.last_time.slice(11, 16) };
}

/** 黄条文案：今天 / 其它日期两形态；无重复返回 ""（弹窗据此隐藏黄条） */
export function dupWarningText(
  dup: { count: number; lastTime: string } | null,
  dateIso: string,
  todayIsoStr: string,
  actionName: string,
): string {
  if (dup === null) return "";
  const when = dateIso === todayIsoStr
    ? "今天"
    : `${+dateIso.slice(5, 7)}月${+dateIso.slice(8, 10)}日`;
  const recent = dup.count > 1 ? `（最近 ${dup.lastTime}）` : `（${dup.lastTime}）`;
  return `${when}已有 ${dup.count} 条${actionName}记录${recent}，请确认不是重复操作。`;
}
```

（buildMarkers 对缺名操作跳过是**有意防御**：名字表由调用方负责全量，见 Task 5 弹窗侧用 colony.actions 全量名表。）

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm vitest run src/lib/monthview.test.ts` → PASS

- [ ] **Step 5: QuickLogDialog 接入（先写失败测试再改）**

`src/components/QuickLogDialog.test.ts` 追加（文件顶部已有 invokeMock 体系）：

```ts
describe("重复提醒（交互第三轮 #8）", () => {
  it("选中日已有同操作记录 → 黄条提醒，但仍可提交", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "colony_month_records") {
        // 用本地 todayIso()（自纠：toISOString 是 UTC，凌晨跑会差一天导致黄条不出现）
        return [{ day: +todayIso().slice(8, 10), action_id: 2, count: 1, last_time: `${todayIso()} 14:32:00` }];
      }
      if (cmd === "log_care") return 9;
      return null;
    });
    const w = mountDlg();
    await flushPromises();
    expect(w.find(".dup-warn").exists()).toBe(true);
    expect(w.find(".dup-warn").text()).toContain("已有 1 条");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(call).toBeDefined(); // 提醒不拦提交
  });

  it("无重复记录不出黄条", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "colony_month_records" ? [] : cmd === "log_care" ? 9 : null,
    );
    const w = mountDlg();
    await flushPromises();
    expect(w.find(".dup-warn").exists()).toBe(false);
  });

  it("标记：当天有其它操作 → 灰点渲染且 tip 带操作名（复审 #1/#2 集成断言）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "colony_month_records") {
        return [{ day: +todayIso().slice(8, 10), action_id: 3, count: 1, last_time: `${todayIso()} 09:00:00` }];
      }
      return null;
    });
    const w = mountDlg();
    await flushPromises();
    await w.find(".dp-trigger").trigger("click");
    const grey = w.find(".dp-day .dot:not(.cur)");
    expect(grey.exists()).toBe(true); // 名字表来自 colony.actions 全量 → 巢穴保湿不丢
    expect((grey.element.closest(".dp-day") as HTMLElement).title).toContain("巢穴保湿");
  });

  it("时间快捷键 −10分 生效（受控 v-model 断言 emit 值）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "colony_month_records" ? [] : null));
    const w = mountDlg();
    await flushPromises();
    const field = w.findComponent(DateTimeField);
    const before = field.props("modelValue") as string;
    await w.find(".dtf-m10").trigger("click");
    const emitted = field.emitted("update:modelValue");
    expect(emitted).toBeTruthy();
    expect(emitted!.at(-1)![0]).toBe(shiftMinutes(before, -10)); // 复审 #7：props 必随父 state 更新，断 emit 值
  });
});
```

（测试顶部需 `import DateTimeField from "./DateTimeField.vue";`、`import { shiftMinutes } from "../lib/calendar";`、`import { todayIso } from "../lib/dates";`；顶部 colony fixture 的 `actions: []` 补第二项 `{ action_id: 3, name: "巢穴保湿", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, days_since_last: null, overdue: false, foods: [] }`（灰点集成断言的名字来源）；既有「提交」用例里的 `invokeMock.mockResolvedValue(3)` 会让 `colony_month_records` 也返回 3——**必须**在这些既有用例的 mock 里显式给出 `colony_month_records: []` 分支，或把 `mockResolvedValue` 改为按命令分发。执行时逐例修。）

`src/components/QuickLogDialog.vue` 修改：

script 部分追加：

```ts
import DateTimeField from "./DateTimeField.vue";
import { buildMarkers, duplicateInfo, dupWarningText } from "../lib/monthview";
import { todayIso } from "../lib/dates";
import type { MonthDayRecords } from "../types";

const monthRows = ref<MonthDayRecords[]>([]);
const viewMonth = ref({ year: +time.value.slice(0, 4), month: +time.value.slice(5, 7) });
let monthSeq = 0; // 复审 #10：快速翻月旧响应后到会污染标记/黄条，序号守卫丢弃过期响应

async function loadMonth(y: number, m: number) {
  viewMonth.value = { year: y, month: m };
  const seq = ++monthSeq;
  try {
    const res = await invoke<MonthDayRecords[]>("colony_month_records", {
      colonyId: props.colony.id,
      year: y,
      month: m,
    });
    if (seq !== monthSeq) return; // 过期响应，丢弃
    monthRows.value = res;
  } catch {
    if (seq === monthSeq) monthRows.value = []; // 标记是增强，失败静默（提交校验权威在后端）
  }
}
onMounted(() => void loadMonth(viewMonth.value.year, viewMonth.value.month));

// 复审 #15/#16：「现在/±10分/选日期」可跨月，月份跟随时间值重同步（黄条判定依赖 viewMonth）
watch(
  () => time.value.slice(0, 7),
  (ym, old) => {
    if (ym !== old) void loadMonth(+ym.slice(0, 4), +ym.slice(5, 7));
  },
);

/** 日历标记：当前操作橙点、其它操作灰点——名字表必须全量（colony.actions），
 * 否则 buildMarkers 跳过缺名操作、灰点与 tip 整体失效（复审 #1/#2） */
const markers = computed(() => {
  const names = new Map<number, string>(props.colony.actions.map((a) => [a.action_id, a.name]));
  names.set(props.action.action_id, props.action.name); // 兜底：当前操作不在 actions 里也不丢橙点
  return buildMarkers(monthRows.value, props.action.action_id, names);
});

/** 选中日期重复判定（时间字段前 10 位 = 日期） */
const dup = computed(() =>
  duplicateInfo(monthRows.value, viewMonth.value.year, viewMonth.value.month, time.value.slice(0, 10), props.action.action_id),
);
const dupText = computed(() => dupWarningText(dup.value, time.value.slice(0, 10), todayIso(), props.action.name));
```

（`import { computed, onMounted, ref, watch } from "vue"` 合并进现有 vue import。）

template：`<input v-model="time" class="time-input" type="datetime-local" />` 替换为：

```html
      <DateTimeField v-model="time" :markers="markers" @month="(v) => void loadMonth(v.year, v.month)" />
      <p v-if="dupText !== ''" class="dup-warn">⚠ {{ dupText }}<span class="why">确属再次操作可直接记录</span></p>
```

style 追加：

```css
.dup-warn {
  margin-top: 6px; padding: 7px 10px; border-radius: 9px;
  background: #fdf1de; border: 1px solid #f3d9a8; color: #b45309;
  font-size: 12px; display: flex; gap: 6px; align-items: baseline;
}
.dup-warn .why { margin-left: auto; font-size: 11px; opacity: 0.8; white-space: nowrap; }
```

- [ ] **Step 6: FeedDialog 同款接入**

`src/components/FeedDialog.vue`：与 QuickLogDialog 完全同构——`<input ... type="datetime-local" />` → `<DateTimeField v-model="time" :markers="markers" @month="..." />` + `.dup-warn`；script 追加同 QuickLog 的 loadMonth/markers/dup 三段（`props.action` 换成 FeedDialog 的 action prop 名，以其现文件为准：它接收 `action: ColonyAction`）。**markers 名字表同 QuickLog 用全量 `props.colony.actions` + 当前操作兜底（复审 #1/#2，勿只装当前操作）；loadMonth 带序号守卫、time 月份 watch 重同步（复审 #10/#15）同款照抄。**

`src/App.test.ts` 喂食 describe 追加一个黄条用例（沿用 App 级 mock 体系）：

```ts
  it("喂食弹窗：今天已有喂食记录 → 黄条提醒但不拦提交（交互第三轮 #8）", async () => {
    colony1With([feedCustom]);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "colony_month_records") {
        return [{ day: +todayIso().slice(8, 10), action_id: 9, count: 1, last_time: `${todayIso()} 08:00:00` }];
      }
      if (cmd === "log_care") return 5;
      if (cmd === "list_colonies") return currentColonies;
      if (cmd === "list_locations") return locations;
      if (cmd === "list_foods") return foods;
      return null;
    });
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="9"]').trigger("click");
    await flushPromises();
    const dlg = wrapper.find(".feed-dialog");
    expect(dlg.find(".dup-warn").text()).toContain("已有 1 条");
    await dlg.findAll(".food")[0].trigger("click");
    await dlg.find(".record-btn").trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "log_care")).toBe(true);
  });
```

**同时必须**：`src/App.test.ts` 的 `baseMock()` switch 加 `case "colony_month_records": return [];`，否则所有打卡/喂食用例拿到 `default: null` 使 `monthRows.value = null` 崩 computed。既有用例里 `.time-input` 的 `setValue`（两处：`App.test.ts:956`、`App.test.ts:1005`）改为：

```ts
await dialog.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-17T21:00");
```

（文件顶 import `DateTimeField from "./components/DateTimeField.vue";`）

- [ ] **Step 7: 全量验证 + Commit**

Run: `pnpm test` → 全绿；`cd src-tauri && cargo test` → 全绿。

```bash
git add src/lib/monthview.ts src/lib/monthview.test.ts src/components/QuickLogDialog.vue src/components/QuickLogDialog.test.ts src/components/FeedDialog.vue src/App.test.ts
git commit -m "feat(ui): 打卡/喂食弹窗接入标记日历DateTimeField+当天重复黄条提醒(不拦提交)

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Rust——list_logs 加 location_id 筛选 + LogRow.location_name

**Files:**
- Modify: `src-tauri/src/care.rs`（LogFilter/LogRow/list_logs + 测试）、`src/types.ts`、`src/lib/loglist.ts`、`src/lib/loglist.test.ts`
- Test: care.rs tests、loglist.test.ts

**Interfaces:**
- Consumes: 既有 list_logs 测试模式
- Produces: `LogFilter.location_id: Option<i64>`（前端 `location_id`）；`LogRow.location_name: Option<String>`（null = 未分组）；TS 侧同步两字段

- [ ] **Step 1: 写失败测试（care.rs tests 追加）**

```rust
    // ── 地点筛选 + 行带地点名（交互第三轮 #1/#5）──
    // （种子依据：db.rs seeds_two_locations 预置 '家'/'公司'，loc_id 按名取不会踩空）

    fn colony_in_loc(conn: &Connection, name: &str, loc: Option<i64>) -> i64 {
        conn.execute(
            "INSERT INTO colony (name, location_id, start_date, status) VALUES (?1, ?2, '2026-01-01', 'active')",
            params![name, loc],
        ).expect("建窝失败");
        conn.last_insert_rowid()
    }
    fn loc_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM location WHERE name = ?1", params![name], |r| r.get(0)).expect("查地点失败")
    }

    #[test]
    fn list_logs_filters_by_location_and_joins_location_name() {
        let conn = mem_conn();
        let home = loc_id(&conn, "家");
        let c1 = colony_in_loc(&conn, "大头一号", Some(home));
        let c2 = colony_in_loc(&conn, "游民", None);
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c2, "喂食", "2026-09-16 20:00:00");

        let page = list_logs(&conn, &LogFilter { location_id: Some(home), ..Default::default() }).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].colony_name, "大头一号");
        assert_eq!(page.rows[0].location_name.as_deref(), Some("家"));

        // 未分组的窝：location_name = None；按「全部」查两行都在
        let all = list_logs(&conn, &LogFilter::default()).unwrap();
        assert_eq!(all.total, 2);
        let nomad = all.rows.iter().find(|r| r.colony_name == "游民").unwrap();
        assert_eq!(nomad.location_name, None);
    }

    #[test]
    fn list_logs_location_and_colony_filters_compose() {
        let conn = mem_conn();
        let home = loc_id(&conn, "家");
        let c1 = colony_in_loc(&conn, "家A", Some(home));
        let c2 = colony_in_loc(&conn, "家B", Some(home));
        log(&conn, c1, "喂食", "2026-09-17 20:00:00");
        log(&conn, c2, "喂食", "2026-09-16 20:00:00");

        let page = list_logs(&conn, &LogFilter {
            location_id: Some(home),
            colony_id: Some(c1),
            ..Default::default()
        }).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].colony_name, "家A");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test list_logs_filters_by_location` → FAIL（LogFilter 无 location_id 字段）

- [ ] **Step 3: 实现 care.rs**

`LogFilter` 加字段（在 `colony_id` 之前，紧挨）：

```rust
    /// 地点筛选（交互第三轮 #1）：窝的所属地点；与 colony_id 组合生效
    #[serde(default)]
    pub location_id: Option<i64>,
```

`LogRow` 结构体加字段（`colony_name` 之后）：

```rust
    /// 窝所属地点名（交互第三轮 #5）；未分组 = None
    pub location_name: Option<String>,
```

`list_logs` 内：

```rust
    if let Some(location_id) = filter.location_id {
        args.push(location_id.into());
        wheres.push(format!("c.location_id = ?{}", args.len()));
    }
```

COUNT 查询改为（原来无 JOIN，现在地点条件引用 c 别名，必须带上 JOIN）：

```rust
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM care_log l JOIN colony c ON c.id = l.colony_id {where_sql}"),
            rusqlite::params_from_iter(args.iter()),
            |row| row.get(0),
        )
        .map_err(db_err)?;
```

SELECT 语句改为（加 `lo.name` 列 + LEFT JOIN location）：

```rust
            "SELECT l.id, l.colony_id, c.name, lo.name, l.action_id, a.name, l.occurred_at, l.note, l.created_at
             FROM care_log l
             JOIN colony c ON c.id = l.colony_id
             LEFT JOIN location lo ON lo.id = c.location_id
             JOIN care_action a ON a.id = l.action_id
             {where_sql}
             ORDER BY l.occurred_at DESC, l.id DESC
             LIMIT {limit} OFFSET {offset}"
```

query_map 闭包与后续解构同步（多一个 `Option<String>`，注意它排在 `colony_name` 后）：

```rust
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
```

循环解构与 `out.push(LogRow { ... })` 相应加 `location_name` 字段。

**既有用例排查**：care.rs 里已有 list_logs 测试构造 `LogRow` 吗？——测试只调函数不构造返回体，无破坏；但若有 `assert_eq!(page.rows[0], LogRow {...})` 之类整结构体比较则需补字段（执行时以编译错误为准逐个修）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test list_logs && cargo test` → PASS

- [ ] **Step 5: 前端类型与纯函数同步**

`src/types.ts`：`LogFilter` 加 `location_id: number | null;`（colony_id 上方）；`LogRow` 加 `location_name: string | null;`（colony_name 下方，注释「未分组 = null」）。

`src/lib/loglist.ts`：

```ts
/** 筛选表单原始态（空串 = 未填，null = 未选） */
export interface LogFilterForm {
  locationId: number | null;
  colonyId: number | null;
  actionId: number | null;
  start: string;
  end: string;
  keyword: string;
}

export function emptyFilterForm(): LogFilterForm {
  return { locationId: null, colonyId: null, actionId: null, start: "", end: "", keyword: "" };
}
```

`buildLogFilter` 返回对象加 `location_id: form.locationId,`。

新增级联纯函数（放文件末尾）：

```ts
/** 地点→窝 级联（交互第三轮 #1）：选了地点，窝下拉只列该地点的窝 */
export function colonyOptionsFor(colonies: Colony[], locationId: number | null): Colony[] {
  if (locationId === null) return colonies;
  return colonies.filter((c) => c.location_id === locationId);
}
```

（顶部 import 追加 `Colony` 类型。）

`src/lib/loglist.test.ts`：既有 `emptyFilterForm()` / `buildLogFilter` 断言全部补 `locationId: null` / `location_id: null`；新增：

```ts
it("colonyOptionsFor：级联过滤（交互第三轮 #1）", () => {
  const cols = [
    { id: 1, location_id: 1 } as Colony,
    { id: 2, location_id: 2 } as Colony,
    { id: 3, location_id: null } as Colony,
  ];
  expect(colonyOptionsFor(cols, 1).map((c) => c.id)).toEqual([1]);
  expect(colonyOptionsFor(cols, null).map((c) => c.id)).toEqual([1, 2, 3]);
});
```

- [ ] **Step 6: 验证 + Commit**

Run: `pnpm vitest run src/lib/loglist.test.ts && pnpm test`（LogListPage.test.ts 此时仍全绿——它走 `toHaveBeenCalledWith` 期望对象会因 filter 多了 location_id 而 **FAIL**：这些期望在本任务一并更新，把各期望对象的 filter 里加 `location_id: null`。改完跑全量。）

```bash
git add src-tauri/src/care.rs src/types.ts src/lib/loglist.ts src/lib/loglist.test.ts src/components/LogListPage.test.ts
git commit -m "feat(backend): list_logs支持location_id筛选+LogRow带location_name；前端级联纯函数

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: LogListPage 改版——地点级联、即改即查、防抖、地点小字、编辑弹窗换 DateTimeField

**Files:**
- Modify: `src/components/LogListPage.vue`、`src/components/LogListPage.test.ts`
- Test: `src/components/LogListPage.test.ts`

**Interfaces:**
- Consumes: `colonyOptionsFor`（Task 6）、`DatePickerPop` / `DateTimeField`（Task 2/3）、`buildMarkers` / `duplicateInfo` / `dupWarningText`（Task 5）、`LocationItem`
- Produces: 无对外新接口（页面组件）

- [ ] **Step 1: 更新测试到目标行为（先失败）**

`src/components/LogListPage.test.ts` 改造要点（执行时逐条落）：

1. mock `list_locations`（挂载 Promise.all 多拉一个命令）。
2. 「筛选组合生效」用例：删除 `.apply-btn` 点击，改为每个 `setValue` 后即断言触发 `list_logs`（select 的 change / DatePickerPop 的 emit）；期望 filter 对象含 `location_id`。
3. 新增级联用例：

```ts
it("地点→窝级联：选地点后窝下拉只剩该地点的窝（交互第三轮 #1）", async () => {
  const wrapper = await mountPage();
  await wrapper.find(".f-location").setValue("1");
  await flushPromises();
  const opts = wrapper.findAll(".f-colony option").map((o) => (o.element as HTMLOptionElement).value);
  expect(opts).toEqual(["", "101"]); // 假数据：窝101在家、102在公司（以文件内造数为准）
  // 且地点变更本身已触发一次查询
  const last = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs").at(-1);
  expect((last![1] as { filter: { location_id: number | null } }).filter.location_id).toBe(1);
});
```

4. 新增防抖用例：

```ts
it("关键词 300ms 防抖自动查询（交互第三轮 #4）", async () => {
  vi.useFakeTimers();
  try {
    const wrapper = await mountPage();
    invokeMock.mockClear();
    await wrapper.find(".f-keyword").setValue("面包虫");
    await vi.advanceTimersByTimeAsync(299);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(false);
    await vi.advanceTimersByTimeAsync(2);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

it("防抖挂起时点「重置」：挂起回调被取消，旧关键词不回写（复审 #8/#9）", async () => {
  vi.useFakeTimers();
  try {
    const wrapper = await mountPage();
    await wrapper.find(".f-keyword").setValue("面包虫");
    invokeMock.mockClear();
    await wrapper.find(".reset-btn").trigger("click");
    await flushPromises();
    await vi.advanceTimersByTimeAsync(400);
    expect((wrapper.find(".f-keyword").element as HTMLInputElement).value).toBe("");
    const kws = invokeMock.mock.calls
      .filter(([cmd]) => cmd === "list_logs")
      .map(([, a]) => (a as { filter: { note_keyword: string | null } }).filter.note_keyword);
    expect(kws).toEqual([null]); // 只有重置那一次查询，且无旧词回写
  } finally {
    vi.useRealTimers();
  }
});
```

5. 新增地点小字用例：

```ts
it("窝名下挂地点小字（交互第三轮 #5）", async () => {
  const wrapper = await mountPage();
  const cell = wrapper.find(".c-colony");
  expect(cell.find(".loc").text()).toBe("家"); // 行数据 location_name="家"
});
```

6. 编辑弹窗 `.time-input` setValue → `dlg.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-10T08:30")`；补 `colony_month_records` mock 返回 `[]`。
7. 删除「时间范围倒置」用例中 `.apply-btn` 相关触发方式：改为设置完起止日期后断言 `.filter-error` 出现且未发查询（设置 start/end 走 DatePickerPop emit 即时触发）。

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/components/LogListPage.test.ts` → FAIL（新行为未实现）

- [ ] **Step 3: 实现 LogListPage.vue**

script 修改（按块）：

```ts
import DatePickerPop from "./DatePickerPop.vue";
import DateTimeField from "./DateTimeField.vue";
import { buildMarkers, duplicateInfo, dupWarningText } from "../lib/monthview";
import { colonyOptionsFor } from "../lib/loglist";
import { todayIso } from "../lib/dates";
import type { LocationItem, MonthDayRecords } from "../types";

const locations = ref<LocationItem[]>([]);
const form = ref<LogFilterForm>(emptyFilterForm()); // LogFilterForm 已含 locationId（Task 6）

/** 级联窝选项（交互第三轮 #1） */
const colonyOptions = computed(() => colonyOptionsFor(colonies.value, form.value.locationId));

function onLocationChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.locationId = v === "" ? null : Number(v);
  // 原选中窝不在新地点 → 清空为「全部」
  if (form.value.colonyId !== null && !colonyOptions.value.some((c) => c.id === form.value.colonyId)) {
    form.value.colonyId = null;
  }
  applyFilters();          // 即改即查（#4）
}

function onColonyChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.colonyId = v === "" ? null : Number(v);
  applyFilters();
}

function onActionChange(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  form.value.actionId = v === "" ? null : Number(v);
  applyFilters();
}

function onDatePick(which: "start" | "end", iso: string) {
  form.value[which] = iso;
  applyFilters();
}

/** 关键词防抖 300ms（#4）：输入停顿即查，不再依赖回车/按钮 */
let kwTimer: ReturnType<typeof setTimeout> | undefined;
function onKeywordInput(e: Event) {
  const v = (e.target as HTMLInputElement).value;
  clearTimeout(kwTimer);
  kwTimer = setTimeout(() => {
    form.value.keyword = v;
    applyFilters();
  }, 300);
}
onBeforeUnmount(() => clearTimeout(kwTimer));
```

（`import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";` 更新既有 import。`applyFilters`/`resetFilters` 保留原名：`applyFilters` = 原「点查询」逻辑即校验+load(0)，现在被各变更入口调用；`resetFilters` **函数体首行加 `clearTimeout(kwTimer)`**（复审 #8/#9：防抖挂起时点重置，300ms 后旧回调会把旧关键词写回 form 再查一次、输入框跳回旧词），末尾 `form.value = emptyFilterForm()` 已覆盖 locationId。）

`onMounted` 的 `Promise.all` 加 `invoke<LocationItem[]>("list_locations")`。

编辑弹窗重复提醒（#8 编辑场景 Q10-C）：script 追加

```ts
const editMonthRows = ref<MonthDayRecords[]>([]);
const editViewMonth = ref({ year: 2026, month: 9 });
let editMonthSeq = 0; // 复审 #10：翻月竞态守卫，同 loadMonth 模式
const editMarkers = computed(() => {
  const names = new Map(actions.value.map((a) => [a.id, a.name]));
  return buildMarkers(editMonthRows.value, editActionId.value ?? -1, names);
});
const editDup = computed(() =>
  editing.value === null || editActionId.value === null
    ? null
    : duplicateInfo(editMonthRows.value, editViewMonth.value.year, editViewMonth.value.month,
        editTime.value.slice(0, 10), editActionId.value),
);
const editDupText = computed(() => {
  if (editing.value === null) return "";
  const actionName = actions.value.find((a) => a.id === editActionId.value)?.name ?? "该操作"; // 复审 #17：查真名
  return dupWarningText(editDup.value, editTime.value.slice(0, 10), todayIso(), actionName);
});

async function loadEditMonth(y: number, m: number) {
  editViewMonth.value = { year: y, month: m };
  if (editing.value === null) return;
  const seq = ++editMonthSeq;
  try {
    const res = await invoke<MonthDayRecords[]>("colony_month_records", {
      colonyId: editing.value.colony_id,
      year: y,
      month: m,
    });
    if (seq !== editMonthSeq) return; // 过期响应丢弃（复审 #10）
    editMonthRows.value = res;
  } catch {
    if (seq === editMonthSeq) editMonthRows.value = [];
  }
}

// 复审 #15：编辑时间跨月时月份重同步
watch(
  () => editTime.value.slice(0, 7),
  (ym, old) => {
    if (ym !== old) void loadEditMonth(+ym.slice(0, 4), +ym.slice(5, 7));
  },
);
```

`openEdit` 末尾追加：`void loadEditMonth(+editTime.value.slice(0, 4), +editTime.value.slice(5, 7));`

template 修改（整段 filters 替换 + 窝列 + 编辑时间控件）：

```html
      <div class="filters">
        <label class="f-label">地点</label>
        <select class="f-location" :value="form.locationId ?? ''" @change="onLocationChange">
          <option value="">全部</option>
          <option v-for="l in locations" :key="l.id" :value="l.id">{{ l.name }}</option>
        </select>

        <label class="f-label">窝</label>
        <select class="f-colony" :value="form.colonyId ?? ''" @change="onColonyChange">
          <option value="">全部</option>
          <option v-for="c in colonyOptions" :key="c.id" :value="c.id">{{ c.name }}</option>
        </select>

        <label class="f-label">操作</label>
        <select class="f-action" :value="form.actionId ?? ''" @change="onActionChange">
          <option value="">全部</option>
          <option v-for="a in enabledActions" :key="a.id" :value="a.id">{{ a.name }}</option>
        </select>

        <label class="f-label">从</label>
        <DatePickerPop :model-value="form.start" placeholder="开始日期" @update:model-value="(v) => onDatePick('start', v)" />
        <label class="f-label">到</label>
        <DatePickerPop :model-value="form.end" placeholder="结束日期" @update:model-value="(v) => onDatePick('end', v)" />

        <input class="f-keyword" :value="form.keyword" type="search" placeholder="搜备注 · 输入即查" @input="onKeywordInput" />
        <button class="reset-btn" type="button" @click="resetFilters">重置</button>
        <span class="auto-note">⚡ 条件变更即查询</span>
      </div>
```

（删除 `apply-btn` 按钮、原 `@keyup.enter`。style 里 `.apply-btn` 规则删除，加 `.auto-note { font-size: 11px; color: var(--accent-deep); background: var(--accent-soft); padding: 1px 10px; border-radius: 999px; white-space: nowrap; }`。）

表格窝列：

```html
              <td class="c-colony">{{ r.colony_name }}<span class="loc">{{ r.location_name ?? "未分组" }}</span></td>
```

style：`.c-colony .loc { display: block; font-size: 11px; font-weight: 400; color: var(--muted); }`

编辑弹窗时间行替换：

```html
          <div class="field-label">发生时间（可补录）</div>
          <DateTimeField v-model="editTime" :markers="editMarkers" @month="(v) => void loadEditMonth(v.year, v.month)" />
          <p v-if="editDupText !== ''" class="dup-warn">⚠ {{ editDupText }}</p>
```

（`.dup-warn` 样式与 QuickLogDialog 相同，复制一份。）

- [ ] **Step 4: 跑测试确认通过 + 全量**

Run: `pnpm vitest run src/components/LogListPage.test.ts` → PASS；`pnpm test` → 全绿。

- [ ] **Step 5: Commit**

```bash
git add src/components/LogListPage.vue src/components/LogListPage.test.ts
git commit -m "feat(logs): 记录页地点筛选级联+即改即查(关键词防抖300ms去查询按钮)+窝列地点小字+编辑弹窗标记日历黄条

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: 首页紧凑卡片（方向一，mock C1）

**Files:**
- Modify: `src/components/ColonyCard.vue`、`src/App.vue`、`src/App.test.ts`
- Test: `src/App.test.ts`

**Interfaces:**
- Consumes: 现有 ColonyCard props/emits（`colony` / `edit` / `saved`）不变
- Produces: 卡片 DOM 类名调整（`.tile` 单行 chip 化、`.card-menu` v-show 菜单内保留 `.edit-btn/.hib-btn/.wake-btn/.past-btn` 原类名——既有测试直接点这些按钮不破坏）

- [ ] **Step 1: 更新 App.test.ts 期望（先失败）**

修改点：

1. 「按地点分组…」用例（原 142-161 行）：
   - 删 `expect(home.text()).toContain("开始饲养 2026-01-20");`
   - `expect(home.text()).toContain("已饲养 / 天")` 改为紧凑形态断言：`expect(home.find(".daysbox").text()).toContain("241");`（不再断言「已饲养 / 天」文案）
2. 「没有任何窝时…」用例（原 193 行）：`.new-colony` → `.new-top-btn`（顶栏按钮），断言 `wrapper.find(".topbar .new-top-btn").exists()`。
3. 新增菜单用例：

```ts
  it("紧凑卡片：⋯ 菜单默认隐藏，点开可见编辑/冬眠入口（交互第三轮 #7）", async () => {
    const wrapper = await mountApp();
    const card = wrapper.find('.card[data-colony-id="1"]');
    const menu = card.find(".card-menu");
    expect((menu.element as HTMLElement).style.display).toBe("none");
    await card.find(".dots").trigger("click");
    expect((menu.element as HTMLElement).style.display).not.toBe("none");
    expect(menu.find(".edit-btn").exists()).toBe(true);
  });
```

4. 其余直接触发 `.edit-btn/.hib-btn/.wake-btn/.past-btn/.resched-btn` 的用例**不改**（v-show 保 DOM，trigger 对隐藏元素同样生效）——这是本设计的验收点。

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/App.test.ts` → FAIL（紧凑形态未实现）

- [ ] **Step 3: 改 ColonyCard.vue（template + style 重写，script 仅加 menu 开关）**

script 追加：

```ts
const menuOpen = ref(false);
function toggleMenu() {
  menuOpen.value = !menuOpen.value;
}
/** 菜单动作执行即收菜单（复审 #12）；点卡片外也收 */
function menuAction(fn: () => void) {
  menuOpen.value = false;
  fn();
}
function onDocClick() {
  menuOpen.value = false;
}
onMounted(() => document.addEventListener("click", onDocClick));
onBeforeUnmount(() => document.removeEventListener("click", onDocClick));
```

（vue import 扩为 `import { computed, onBeforeUnmount, onMounted, ref } from "vue";`。）

template 全量替换（保持既有类名/结构语义，紧凑化）：

```html
<template>
  <article class="card" :class="{ hib: hibernating }" :data-colony-id="colony.id">
    <div class="chead">
      <span class="cname">{{ colony.name }}</span>
      <span v-if="colony.species" class="chip sp">{{ colony.species }}</span>
      <span class="chip st" :class="{ hib: colony.status === 'hibernating' }">
        {{ STATUS_TEXT[colony.status] }}
      </span>
      <span class="daysbox"><span class="n">{{ colony.days_raised }}</span> <span class="l">天</span></span>
    </div>

    <div v-if="banner" class="banner" data-testid="hib-banner">
      {{ banner.line }}
      <span v-if="banner.nearWake" class="chip wake">临近出眠</span>
      <button
        class="resched-btn"
        type="button"
        title="修改预计出眠日：未发的临近/出眠提醒按新日期重算"
        @click="openHibernation('edit')"
      >
        改期
      </button>
    </div>

    <div class="tiles">
      <button
        v-for="{ action: a, view } in tiles"
        :key="a.action_id"
        class="tile"
        :class="view.tone"
        :data-action-id="a.action_id"
        type="button"
        :title="a.is_feeding && a.foods.length > 0 ? feedingTooltip(a.foods) : undefined"
        @click="onTile(a)"
      >
        <span v-if="a.icon" class="t-ico">{{ a.icon }}</span>
        <span class="t-name">{{ a.name }}</span>
        <span v-if="a.kind === 'log_only'" class="t-tag">仅登记</span>
        <span class="pill">{{ view.text }}</span>
      </button>
    </div>

    <div class="foot">
      <span class="recent">{{ recentLine }}</span>
      <button class="dots" type="button" title="编辑 / 冬眠等更多操作" @click.stop="toggleMenu">⋯</button>
    </div>

    <!-- 交互第三轮 #7：低频操作收进 ⋯ 菜单（v-show 保 DOM，选择器与测试不破坏） -->
    <div v-show="menuOpen" class="card-menu" @click.stop>
      <button v-if="colony.status === 'active'" class="m-item hib-btn" type="button" @click="menuAction(() => openHibernation('start'))">
        ❄ 开始冬眠
      </button>
      <button v-if="hibernating" class="m-item wake-btn" type="button" @click="menuAction(() => openHibernation('wake'))">
        ☀ 确认出眠
      </button>
      <button
        v-if="colony.status !== 'ended'"
        class="m-item past-btn"
        type="button"
        title="补录已闭合的过去冬眠段"
        @click="menuAction(() => openHibernation('past'))"
      >
        📅 补录冬眠
      </button>
      <button class="m-item edit-btn" type="button" @click="menuAction(() => $emit('edit'))">✏️ 编辑窝信息</button>
    </div>

    <!-- 三个弹窗组件原样保留（FeedDialog/QuickLogDialog/HibernationDialog） -->
    <FeedDialog v-if="showFeed && feedAction !== null" …原属性不变… />
    <QuickLogDialog v-if="showQuick && quickAction !== null" … />
    <HibernationDialog v-if="showHibernation" … />
  </article>
</template>
```

（执行时弹窗三段从原文件原样拷贝，不要省略属性。）

style 重写（删除被替换的旧规则，新增/修改如下；弹窗相关样式原文件没有——弹窗样式在各自组件内，无需搬）：

```css
.card { position: relative; background: var(--card); border: 1px solid var(--border);
  border-radius: 12px; padding: 10px 12px; box-shadow: var(--shadow); }
.card.hib { background: linear-gradient(180deg, var(--hib-soft), var(--card) 60%); }
.chead { display: flex; align-items: center; gap: 8px; }
.cname { font-size: 15px; font-weight: 700; }
.chips { display: contents; }            /* 兼容：不再需要容器 */
.chip { font-size: 11px; padding: 0 8px; border-radius: 999px; border: 1px solid transparent; }
.chip.sp { background: var(--accent-soft); color: var(--accent-deep); }
.chip.st { background: var(--ok-soft); color: var(--ok); }
.chip.st.hib { background: var(--hib-soft); color: var(--hib); }
.chip.wake { background: var(--accent-soft); color: var(--accent-deep); font-weight: 600; }
.daysbox { margin-left: auto; white-space: nowrap; }
.daysbox .n { font-size: 16px; font-weight: 800; }
.daysbox .l { font-size: 10px; color: var(--muted); }

.banner { margin-top: 8px; padding: 4px 10px; border-radius: 8px; background: var(--hib-soft);
  color: var(--hib); font-size: 12px; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.resched-btn { margin-left: auto; border: 1px solid var(--border); background: var(--card);
  color: var(--hib); font: inherit; font-size: 11px; padding: 0 8px; border-radius: 999px;
  cursor: pointer; white-space: nowrap; }
.resched-btn:hover { border-color: var(--hib); color: var(--text); }

/* 操作块：单行 chip，2 列（mock C1） */
.tiles { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; margin-top: 8px; }
.tile { border: 1px solid var(--border); background: var(--tile); border-radius: 9px;
  padding: 5px 9px; cursor: pointer; font: inherit; color: var(--text); display: flex;
  align-items: center; gap: 6px; font-size: 13px; transition: 0.12s; min-width: 0; }
.tile:hover { border-color: var(--accent); }
.tile .t-ico { font-size: 14px; }
.tile .t-name { font-weight: 600; white-space: nowrap; }
.t-tag { font-size: 10px; font-weight: 400; color: var(--muted); border: 1px solid var(--border);
  padding: 0 5px; border-radius: 999px; white-space: nowrap; }
.pill { margin-left: auto; font-size: 11px; padding: 0 8px; border-radius: 999px; white-space: nowrap; }
.tile.reg .pill, .tile.none .pill { background: var(--tile); color: var(--muted); }
.tile.ok .pill { background: var(--ok-soft); color: var(--ok); }
.tile.bad { border-color: var(--bad); background: var(--bad-soft); }
.tile.bad .pill { background: #fff; color: var(--bad); font-weight: 600; }
.tile.mute { opacity: 0.55; cursor: pointer; }
.tile.mute .pill { background: var(--hib-soft); color: var(--hib); }
.tile:disabled { opacity: 0.6; cursor: default; }

/* 底部行 + ⋯ 菜单 */
.foot { margin-top: 8px; display: flex; align-items: center; gap: 8px; }
.recent { flex: 1; font-size: 11px; color: var(--muted); white-space: nowrap;
  overflow: hidden; text-overflow: ellipsis; }
.dots { flex: none; border: none; background: transparent; color: var(--muted);
  font-size: 16px; cursor: pointer; padding: 0 4px; border-radius: 6px; line-height: 1; }
.dots:hover { background: var(--tile); color: var(--text); }
.card-menu { position: absolute; right: 10px; bottom: 34px; background: var(--card);
  border: 1px solid var(--border-strong); border-radius: 10px;
  box-shadow: 0 8px 30px rgba(60, 50, 30, 0.15); padding: 4px; z-index: 20; min-width: 108px; }
.m-item { display: block; width: 100%; border: none; background: transparent; font: inherit;
  font-size: 12px; color: var(--text); padding: 6px 10px; border-radius: 7px; cursor: pointer;
  text-align: left; white-space: nowrap; }
.m-item:hover { background: var(--accent-soft); color: var(--accent-deep); }
```

- [ ] **Step 4: 改 App.vue 首页区 + 顶栏**

顶栏 `.tools` 内（`⚙ 设置` 前）加：

```html
        <button class="ghost-btn new-top-btn" type="button" @click="openCreate">＋ 新建窝</button>
```

首页区：删除页底 `<button class="new-colony" …>＋ 新建窝（…）</button>`；`.group` 的 `margin-top: 26px` → `14px`（首个 group 处理：样式里 `.group:first-child { margin-top: 0 }` 如无则加）；`.cards` 的 `minmax(330px, 1fr)` → `minmax(300px, 1fr)`；`.group-head h2` 16px → 14px；`gap: 14px` → `10px`。删除 `.new-colony` 样式规则。

- [ ] **Step 5: 验证 + Commit**

Run: `pnpm test` → 全绿（含既有直接点 edit/hib/wake/past 按钮的用例）

```bash
git add src/components/ColonyCard.vue src/App.vue src/App.test.ts
git commit -m "feat(home): 首页紧凑卡片(方向一)——单行chip操作块+天数内联+去开始日期+低频按钮收进⋯菜单+新建窝挪顶栏

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: ColonyFormDialog + HibernationDialog 换素版 DatePickerPop

**Files:**
- Modify: `src/components/ColonyFormDialog.vue`、`src/components/HibernationDialog.vue`、`src/App.test.ts`
- Test: `src/App.test.ts`

**Interfaces:**
- Consumes: `DatePickerPop`（Task 2）
- Produces: 无

- [ ] **Step 1: 更新 App.test.ts（先失败）**

全部原生日期 `setValue` 替换为组件 emit（DatePickerPop 发的是纯日期）：

- 新建窝（原 209 行）：`await dialog.find(".date-input").setValue("2026-09-18");` → `await dialog.findComponent(DatePickerPop).vm.$emit("update:modelValue", "2026-09-18");`
- 编辑窝预填断言（原 253 行）：`(dialog.find(".date-input").element as HTMLInputElement).value` → `dialog.findComponent(DatePickerPop).props("modelValue")`。
- 冬眠各处（`.start-input/.end-input/.actual-input/.past-start-input/.past-end-input` 的 setValue 与 value 读取，原 1122-1124、1148-1153、1184-1185、1213-1214、1240、1242 行）：同一弹窗内多个 DatePickerPop，用 ` findAllComponents(DatePickerPop)[i]` 按模板出现顺序定位（start=0、end=1；wake=actual 0；past=start 0、end 1；edit=end 0），emit / props 对应替换。
- 顶部 import 追加 `import DatePickerPop from "./components/DatePickerPop.vue";`

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run src/App.test.ts` → FAIL（原生 input 已不存在/未换）

- [ ] **Step 3: 改两个弹窗**

`ColonyFormDialog.vue`（原 154 行）：

```html
      <div class="field-label">开始饲养日期 *</div>
      <DatePickerPop v-model="form.startDate" placeholder="开始日期" />
```

（script import DatePickerPop；`form.startDate` 仍是 `YYYY-MM-DD` 字符串，v-model 形态一致；删除原 `.dialog input[type="date"]` 样式中不再命中的部分可留可删——保持 `.dialog input[type="date"]` 规则不破坏其它 input。）

`HibernationDialog.vue`（原 111-138 行）五处同型替换，例如：

```html
        <div class="field-label">开始日期（默认今天）</div>
        <DatePickerPop v-model="startDate" placeholder="开始日期" />
```

**关键行为保持**：开始日变化联动预计结束（+120 天、未手动改过才跟随）——原实现挂在 `@input="endTouched = true"`（预计结束的手动标记）与 startDate 的 watch/联动上。改造后：

- 预计结束的 `endTouched` 标记改挂在 `@update:model-value="endTouched = true"`；
- startDate 联动逻辑照旧（该弹窗 script 内已有 `addDays(start, 120)` 的 watch 或 handler——以现文件为准，把触发源从 input 事件换成 DatePickerPop 的 update:modelValue，逻辑不动）。

（执行时先读 HibernationDialog.vue script 现有联动实现再落，保持测试「改开始日自动 +120、手动改过不覆盖」两条用例语义不变。）

- [ ] **Step 4: 验证 + Commit**

Run: `pnpm test` → 全绿

```bash
git add src/components/ColonyFormDialog.vue src/components/HibernationDialog.vue src/App.test.ts
git commit -m "feat(ui): 新建窝/冬眠弹窗换自绘素版日历——原生date控件全部退场

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: 收尾——全量验证 + README

**Files:**
- Modify: `README.md`

**Interfaces:** 无

- [ ] **Step 1: 全量验证（证据留档）**

Run（三条都要跑、都要绿，输出贴进最终汇报）：

```bash
pnpm test
cd src-tauri && cargo test
pnpm build   # vue-tsc --noEmit + vite build，类型与打包双检
```

- [ ] **Step 2: README 更新**

`README.md` 功能清单追加交互第三轮条目（沿用文件既有行文与测试数格式；测试数以 Step 1 实际输出为准刷新）：
- 记录页：地点筛选（级联窝）、即改即查、窝列地点、自绘日历
- 打卡/喂食/编辑：标记日历 + 当天重复黄条提醒
- 外壳：顶栏固定、内容独立滚动、右键屏蔽、默认窗口 960×680
- 首页：紧凑卡片

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): 交互第三轮8项改进入册+测试数刷新

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 记录

- **规格覆盖**：决策表 8 项 ↔ Task 1(#2#6) / Task 2-3(#3) / Task 4-5(#8) / Task 6(#1#5) / Task 7(#1#4#5#8编辑场景) / Task 8(#7) / Task 9(#3范围=全部6处) / Task 10(验证+文档)。无缺口。
- **占位符扫描**：Task 3 测试选择器统一为 `.dp-trigger`（rev1 已直接写对，无内联矛盾指令）；Task 8 弹窗三段"原样拷贝"给出明确指令与属性来源（原文件），无 TBD。
- **类型一致性**：`MonthDayRecords`（Rust/TS 同名同字段）、`markers: Record<string, {current, tip}>`（Task 2/3/5 一致）、`colonyOptionsFor`（Task 6 定义 / Task 7 消费）、`colony_month_records` invoke 参数 `{colonyId, year, month}`（Task 4/5/7 一致）。
- **风险点**（执行时注意）：① App.test.ts `baseMock` 忘加 `colony_month_records` 分支会让 `default: null` 流进 `monthRows`（已写进 Task 5 Step 6）；② 既有 `mockResolvedValue` 单值 mock 会污染新命令（Task 5 Step 5 已注明逐例改分发）；③ HibernationDialog 的 +120 天联动触发源迁移（Task 9 Step 3 已注明先读现实现）。

## 修订记录 · rev1（夜链复审环，2026-09-19）

消化 codex+pi 盲评 19 条证实项（对照 `.xcheck/20260919-115517/SUMMARY.md`）：

- #1/#2 标记名字表全量化（colony.actions + 当前操作兜底）+ 灰点集成断言（QuickLog 测试补 colony.actions 第二项 fixture）
- #3 stepTimeField 定稿：分钟算术进位、小时 23↔0 回绕、步进不跨日（新增 00:00−1 边界断言）
- #4 buildMarkers 去 sort，按 rows 到达序输出（Rust 侧 ORDER BY 已定序）
- #5 `.dp-day` 断言拆 dim / not-dim
- #6 `.dtf-time` 断言改 `.num` 逐个取
- #7 −10分 用例改断 emit 值（受控 v-model 下 props 必变）
- #8/#9 resetFilters 首行 clearTimeout(kwTimer) + 竞态用例
- #10 loadMonth/loadEditMonth 序号守卫丢弃过期响应
- #11 弹层水平钳制 popShift（右缘放不下时左移）
- #12 菜单动作 menuAction 即收 + document 点外收（dots @click.stop 防开关抵消）
- #13/#14 shell.ts 真幂等（模块级单例，off 后可重装）
- #15/#16 时间值跨月 watch 重同步月份（黄条判定依赖）
- #17 editDupText 查真实操作名
- #18 日期控件计数 6 处 → 11 处（Architecture 段）
- #19 Task 3 提交前补全量 pnpm test；`.dtf-trigger` 选择器直接写对，删除自相矛盾的内联指令
- **自纠（评审两家均未提）**：QuickLog 黄条用例的 `new Date().toISOString()` 是 UTC，东八区凌晨 0–8 点跑测试日期差一天 → 黄条不出现；改用本地 `todayIso()`
- pi#13（seed 无'家'致 loc_id 踩空）查 db.rs:270 **证伪**不改，测试段附种子依据注释

---

## xcheck 评审附录 · 20260919-121817

> **以下为评审参考,以实际执行为准**(验证证据是评审时点的快照,代码可能已演进);
> 但"撞上关注项"的动作不是参考 —— 停下反馈用户,别默默绕过。

两轮盲评（codex + pi）：round 0 共 19 条证实已全部修入本 rev1（见上方修订记录）；round 1 终态**夜间收工**——剩余 8 条已证实项不再开修订轮，全部由下游 spec 固化为需求（`docs/superpowers/specs/2026-09-19-ui-interaction-round3-spec.md`），实施时按 spec 执行即视为消化：

**① 查实的（round 1 剩余 8 条，修法已写进 spec）**
1. `colony_month_records` 会把正在编辑的记录自身计入 → 编辑弹窗误报重复（spec：命令加 `exclude_log_id`）
2. 弹层几何不完整：上下都不足时仍向下溢出、窄窗左溢出（spec：向空间更大侧翻 + 水平双向 clamp）
3. LogListPage `list_logs` 无响应序号守卫，旧查询后到覆盖新结果（spec：load 加 seq）
4. `DateTimeField.pickDate` 空值时 emit `"YYYY-MM-DDT"`（spec：空时兜底 "00:00"）
5. DateTimeField 测试多步断言在无 v-model 回写的 mount 下必失败（spec：单步固定基值断言）
6. `onKeywordInput` 把 form.keyword 更新延迟进防抖回调 → 重置后输入框残留旧词（spec：input 即同步 state、只防抖查询）
7. Self-Review 残留"全部6处"文案（spec 按实际 11 处为准，本处不回改正文）
8. shell.ts 陈旧卸载器可清空模块态致叠监听（spec：off 前校验身份 `if (offShield === off)`）

**② 实验已做的**
无。

**③ 存疑的（开发时要盯）**
- 弹层可能被祖先 overflow 裁剪（当前 CSS 无此祖先；接入真实滚动容器后目检一次弹层表现）——触发：Task 2/7 完成后自测时；命中：停下反馈，别默默绕过。

产物目录 `.xcheck/20260919-115517/`（round 0）与 `.xcheck/20260919-121817/`（round 1）；修订版路径即本文件（rev1，执行基线）。
