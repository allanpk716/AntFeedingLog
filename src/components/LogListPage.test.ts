import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import LogListPage from "./LogListPage.vue";
import DatePickerPop from "./DatePickerPop.vue";
import DateTimeField from "./DateTimeField.vue";
import type { CareActionItem, Colony, FoodItem, LocationItem, LogPage, LogRow } from "../types";

// 不依赖 Tauri 运行时：mock 掉 IPC
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const locations: LocationItem[] = [
  { id: 1, name: "家", enabled: true, sort: 1 },
  { id: 2, name: "公司", enabled: true, sort: 2 },
];

// 大头一号在家、针毛一号在公司：地点→窝级联的造数基础
const colonies: Colony[] = [
  {
    id: 1,
    name: "大头一号",
    species: null,
    location_id: 1,
    start_date: "2026-01-20",
    status: "active",
    days_raised: 241,
    actions: [],
    recent: [],
    hibernation: null,
  },
  {
    id: 2,
    name: "针毛一号",
    species: null,
    location_id: 2,
    start_date: "2026-06-14",
    status: "active",
    days_raised: 96,
    actions: [],
    recent: [],
    hibernation: null,
  },
];

const actions: CareActionItem[] = [
  { id: 1, name: "喂食", icon: null, kind: "reminding", is_feeding: true, suggested_interval_days: 3, enabled: true, sort: 1, referenced: false, is_preset: true },
  { id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 2, referenced: true, is_preset: true },
  { id: 3, name: "降温", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: false, sort: 3, referenced: true, is_preset: false },
  // 撤食（票 03/票 01）：第五种预置操作，性质 follow（跟随喂食）；kind 原样透传字符串，
  // types 未收录时按运行时值处理——页面只依赖 enabled / is_feeding 两个标记位
  { id: 4, name: "撤食", icon: null, kind: "follow" as unknown as CareActionItem["kind"], is_feeding: false, suggested_interval_days: null, enabled: true, sort: 5, referenced: false, is_preset: true },
];

const foods: FoodItem[] = [
  { id: 1, name: "种子", enabled: true, sort: 1, suggested_interval_days: 3, referenced: false, is_preset: true },
  { id: 2, name: "干虾仁", enabled: false, sort: 2, suggested_interval_days: 7, referenced: true, is_preset: true },
  { id: 3, name: "面包虫", enabled: true, sort: 3, suggested_interval_days: 7, referenced: false, is_preset: true },
  { id: 4, name: "糖水", enabled: false, sort: 4, suggested_interval_days: null, referenced: false, is_preset: false },
];

/** 行 101：喂食、在家（地点小字非空路径）、引用了停用食物「干虾仁」；
 * 行 102：换水、未分组（地点小字 null 路径）；行 103 留给「加载更多」 */
const rows: LogRow[] = [
  {
    id: 101,
    colony_id: 1,
    colony_name: "大头一号",
    location_name: "家",
    action_id: 1,
    action_name: "喂食",
    occurred_at: "2026-09-17 21:00:00",
    created_at: "2026-09-17 21:05:00",
    note: "加餐",
    food_ids: [2],
    food_names: ["干虾仁"],
  },
  {
    id: 102,
    colony_id: 2,
    colony_name: "针毛一号",
    location_name: null,
    action_id: 2,
    action_name: "活动区换水",
    occurred_at: "2026-09-16 08:00:00",
    created_at: "2026-09-16 08:00:00",
    note: "",
    food_ids: [],
    food_names: [],
  },
];

const leftover: LogRow[] = [
  {
    id: 103,
    colony_id: 1,
    colony_name: "大头一号",
    location_name: "家",
    action_id: 1,
    action_name: "喂食",
    occurred_at: "2026-09-10 09:00:00",
    created_at: "2026-09-12 09:00:00",
    note: "补记",
    food_ids: [3],
    food_names: ["面包虫"],
  },
];

let currentPage: LogPage = { total: 3, rows };

/** 命令分发底座：编辑弹窗月数据默认空（无重复）；个别用例只覆写关心的分支 */
function baseImpl(cmd: string): unknown {
  switch (cmd) {
    case "list_colonies":
      return colonies;
    case "list_actions":
      return actions;
    case "list_foods":
      return foods;
    case "list_locations":
      return locations;
    case "list_logs":
      return currentPage;
    case "colony_month_records":
      return [];
    default:
      return null;
  }
}

function baseMock() {
  invokeMock.mockImplementation(baseImpl);
}

async function mountPage() {
  const wrapper = mount(LogListPage);
  await flushPromises();
  return wrapper;
}

/** 打开某行的编辑弹窗 */
async function openEdit(wrapper: Awaited<ReturnType<typeof mountPage>>, rowId: number) {
  await wrapper.find(`.log-row[data-log-id="${rowId}"] .edit-btn`).trigger("click");
  await flushPromises();
  return wrapper.find(".edit-dialog");
}

/** 筛选行的从/到两个 DatePickerPop（模板序：[0]=从，[1]=到） */
function filterPickers(wrapper: Awaited<ReturnType<typeof mountPage>>) {
  return wrapper.findAllComponents(DatePickerPop);
}

beforeEach(() => {
  invokeMock.mockReset();
  currentPage = { total: 3, rows };
  baseMock();
});

describe("记录列表页（票 08）", () => {
  it("挂载拉字典（含地点）并发起默认查询：表格渲染时间/窝（地点小字）/操作/食物/备注，total 可见", async () => {
    const wrapper = await mountPage();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    expect(invokeMock).toHaveBeenCalledWith("list_actions");
    expect(invokeMock).toHaveBeenCalledWith("list_foods");
    expect(invokeMock).toHaveBeenCalledWith("list_locations");
    expect(invokeMock).toHaveBeenCalledWith("list_logs", {
      filter: {
        location_id: null,
        colony_id: null,
        action_id: null,
        start: null,
        end: null,
        note_keyword: null,
        limit: 50,
        offset: 0,
      },
    });

    const locOpts = wrapper.findAll(".f-location option").map((o) => o.text());
    expect(locOpts).toEqual(["全部", "家", "公司"]);

    const rows = wrapper.findAll(".log-row");
    expect(rows.length).toBe(2);
    expect(rows[0].find(".c-time").text()).toBe("2026-09-17 21:00");
    expect(rows[0].find(".c-colony").text()).toContain("大头一号");
    expect(rows[0].find(".c-action").text()).toBe("喂食");
    expect(rows[0].find(".c-foods").text()).toContain("干虾仁");
    expect(rows[0].find(".c-note").text()).toBe("加餐");
    // 空备注显示占位
    expect(rows[1].find(".c-note").text()).toBe("—");
    expect(wrapper.find(".total-note").text()).toContain("3");
  });

  it("筛选组合生效：条件变更即查（无查询按钮），组合 filter 直发（验收 1）", async () => {
    const wrapper = await mountPage();
    invokeMock.mockClear();

    await wrapper.find(".f-location").setValue("1");
    await wrapper.find(".f-colony").setValue("1");
    await wrapper.find(".f-action").setValue("1");
    const pickers = filterPickers(wrapper);
    await pickers[0].vm.$emit("update:modelValue", "2026-09-01");
    await pickers[1].vm.$emit("update:modelValue", "2026-09-18");
    await flushPromises();

    // 即改即查：5 次变更恰好 5 次查询
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
    expect(calls.length).toBe(5);
    expect(calls[calls.length - 1][1]).toEqual({
      filter: {
        location_id: 1,
        colony_id: 1,
        action_id: 1,
        start: "2026-09-01",
        end: "2026-09-18",
        note_keyword: null,
        limit: 50,
        offset: 0,
      },
    });
  });

  it("地点→窝级联：选地点后窝下拉只剩该地点的窝；原选中窝不在列清空为「全部」（交互第三轮 #1）", async () => {
    const wrapper = await mountPage();
    invokeMock.mockClear();

    // 先选中窝 1（大头一号·家），再切到「公司」→ 窝不在列，清空为「全部」
    await wrapper.find(".f-colony").setValue("1");
    await flushPromises();
    await wrapper.find(".f-location").setValue("2");
    await flushPromises();
    expect((wrapper.find(".f-colony").element as HTMLSelectElement).value).toBe("");

    const opts = wrapper
      .findAll(".f-colony option")
      .map((o) => (o.element as HTMLOptionElement).value);
    expect(opts).toEqual(["", "2"]); // 公司只有针毛一号

    // 切回「家」→ 只剩大头一号
    await wrapper.find(".f-location").setValue("1");
    await flushPromises();
    const optsHome = wrapper
      .findAll(".f-colony option")
      .map((o) => (o.element as HTMLOptionElement).value);
    expect(optsHome).toEqual(["", "1"]);

    // 地点变更本身即触发查询（尾次带 location_id=1、窝已清空）
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
    const last = calls[calls.length - 1][1] as { filter: { location_id: number | null; colony_id: number | null } };
    expect(last.filter.location_id).toBe(1);
    expect(last.filter.colony_id).toBeNull();
  });

  it("关键词 300ms 防抖自动查询（交互第三轮 #4）", async () => {
    // 只 fake setTimeout/clearTimeout（防抖计时器）：默认全套 fake 连 setImmediate 一起劫持，
    // flushPromises 内部走 setImmediate 会挂死
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const wrapper = await mountPage();
      invokeMock.mockClear();
      await wrapper.find(".f-keyword").setValue("面包虫");
      await vi.advanceTimersByTimeAsync(299);
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(false);
      await vi.advanceTimersByTimeAsync(2);
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(true);
      const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
      const last = calls[calls.length - 1][1] as { filter: { note_keyword: string | null } };
      expect(last.filter.note_keyword).toBe("面包虫");
    } finally {
      vi.useRealTimers();
    }
  });

  it("IME 两段式：组词期不查询，compositionend 补同步后带词查询（交互第三轮 #4 回归）", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }); // 同上：不劫持 setImmediate，flushPromises 会挂死
    try {
      const wrapper = await mountPage();
      invokeMock.mockClear();
      const kw = wrapper.find(".f-keyword");

      // 组词期：拼音片段 input 被守卫挡下——不同步表单、不挂防抖
      await kw.trigger("compositionstart");
      await kw.setValue("mianbaochong");
      await vi.advanceTimersByTimeAsync(400);
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(false);

      // 上屏确认：compositionend 复位守卫并补同步（读 target.value 的最终上屏文本）+ 挂 300ms 防抖
      (kw.element as HTMLInputElement).value = "面包虫";
      await kw.trigger("compositionend");
      await vi.advanceTimersByTimeAsync(301);
      const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
      expect(calls.length).toBe(1); // 组词期零查询，上屏后恰好一次
      const last = calls[calls.length - 1][1] as { filter: { note_keyword: string | null } };
      expect(last.filter.note_keyword).toBe("面包虫");
    } finally {
      vi.useRealTimers();
    }
  });

  it("防抖挂起时点「重置」：挂起回调被取消，旧关键词不回写（复审 #8/#9）", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }); // 同上：不劫持 setImmediate
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

  it("时间范围倒置前端先拦：不发起查询并提示（起始单独有效会即时查一次，倒置组合被拦不再查）", async () => {
    const wrapper = await mountPage();
    const pickers = filterPickers(wrapper);

    await pickers[0].vm.$emit("update:modelValue", "2026-09-10");
    await flushPromises();
    invokeMock.mockClear(); // 清掉起始单独查询；只验证倒置组合不再发查询

    await pickers[1].vm.$emit("update:modelValue", "2026-09-01");
    await flushPromises();

    expect(wrapper.find(".filter-error").text()).toContain("倒置");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(false);
  });

  it("加载更多：offset 递增加入行，取完按钮隐藏", async () => {
    const wrapper = await mountPage();
    // total 3 > 已载 2 应有下一页
    expect(wrapper.find(".more-btn").exists()).toBe(true);

    invokeMock.mockClear();
    currentPage = { total: 3, rows: leftover };
    await wrapper.find(".more-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith(
      "list_logs",
      expect.objectContaining({ filter: expect.objectContaining({ offset: 2 }) }),
    );
    expect(wrapper.findAll(".log-row").length).toBe(3);
    expect(wrapper.find(".more-btn").exists()).toBe(false);
  });

  it("列表查询带响应序号守卫：旧查询后到不得覆盖新结果（复审修正）", async () => {
    let releaseFirst: ((v: LogPage) => void) | null = null;
    let callCount = 0;
    invokeMock.mockImplementation(
      (cmd: string) =>
        new Promise((resolve) => {
          if (cmd === "list_logs") {
            callCount += 1;
            if (callCount === 1) {
              // 第 1 次（默认查询）：挂起，等第 2 次落地后再后到
              releaseFirst = resolve;
              return;
            }
            resolve({ total: 1, rows: leftover }); // 第 2 次：新结果先到
            return;
          }
          resolve(baseImpl(cmd));
        }),
    );
    const wrapper = mount(LogListPage);
    await flushPromises();
    expect(wrapper.findAll(".log-row").length).toBe(0); // 默认查询仍挂起

    await wrapper.find(".f-action").setValue("1"); // 新查询先完成
    await flushPromises();
    expect(wrapper.findAll(".log-row").length).toBe(1);

    releaseFirst!({ total: 3, rows }); // 旧响应后到
    await flushPromises();
    // 旧结果被序号守卫丢弃：列表仍是新查询的结果
    expect(wrapper.findAll(".log-row").length).toBe(1);
    expect(wrapper.find(".total-note").text()).toContain("1");
  });

  it("窝名下挂地点小字：有地点显地点、null 显「未分组」（交互第三轮 #5）", async () => {
    const wrapper = await mountPage();
    const cell101 = wrapper.find('.log-row[data-log-id="101"] .c-colony');
    expect(cell101.find(".loc").text()).toBe("家");
    const cell102 = wrapper.find('.log-row[data-log-id="102"] .c-colony');
    expect(cell102.find(".loc").text()).toBe("未分组");
  });

  it("编辑弹窗预填当前值；停用项标「已停用」：原引用可选、无关停用禁选（规则 10）", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);

    expect(wrapper.findComponent(DateTimeField).props("modelValue")).toBe("2026-09-17T21:00");
    expect((dlg.find(".action-select").element as HTMLSelectElement).value).toBe("1");
    expect((dlg.find(".note-input").element as HTMLInputElement).value).toBe("加餐");

    const chips = dlg.findAll(".food");
    const byName = (name: string) => chips.find((c) => c.text().startsWith(name))!;
    // 停用但被本条引用：可选、带「已停用」标记、默认选中
    const shrimp = byName("干虾仁");
    expect(shrimp.text()).toContain("已停用");
    expect(shrimp.classes()).toContain("selected");
    expect(shrimp.attributes("disabled")).toBeUndefined();
    // 无关停用项：禁选
    expect(byName("糖水").attributes("disabled")).toBeDefined();
    // 启用项正常
    expect(byName("面包虫").attributes("disabled")).toBeUndefined();
  });

  it("编辑弹窗：月数据带 excludeLogId=当前记录 id（复审修正），命中同操作出黄条但不拦保存", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "colony_month_records"
        ? [{ day: 17, action_id: 1, count: 1, last_time: "2026-09-17 21:00:00" }]
        : baseImpl(cmd),
    );
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);

    // 排除自身：colony_month_records 传 excludeLogId=101
    const cmrs = invokeMock.mock.calls.filter(([cmd]) => cmd === "colony_month_records");
    expect(cmrs.length).toBeGreaterThan(0);
    expect(cmrs[cmrs.length - 1][1]).toEqual({ colonyId: 1, year: 2026, month: 9, excludeLogId: 101 });

    // 该月该日已有同操作（喂食）数据 → 黄条出现；保存不被拦
    expect(dlg.find(".dup-warn").exists()).toBe(true);
    expect(dlg.find(".dup-warn").text()).toContain("已有 1 条喂食记录");
    await dlg.find(".save-btn").trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "update_log")).toBe(true);
  });

  it("编辑喂食记录更换食物提交 update_log（验收 2），成功后关窗、重查列表并通知外层重算", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);

    await wrapper.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-10T08:30");
    // 换食物：默认引用的干虾仁点掉，勾上面包虫
    await dlg
      .findAll(".food")
      .find((c) => c.text().startsWith("干虾仁"))!
      .trigger("click");
    await dlg
      .findAll(".food")
      .find((c) => c.text().startsWith("面包虫"))!
      .trigger("click");
    await dlg.find(".note-input").setValue("  改投面包虫  ");

    invokeMock.mockClear();
    await dlg.find(".save-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("update_log", {
      id: 101,
      input: {
        occurred_at: "2026-09-10T08:30",
        action_id: 1,
        food_ids: [3],
        note: "改投面包虫",
      },
    });
    expect(wrapper.find(".edit-dialog").exists()).toBe(false);
    const reloads = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
    expect(reloads.length).toBeGreaterThanOrEqual(1);
    expect(wrapper.emitted("changed")).toBeTruthy();
  });

  it("非喂食行编辑无食物区，提交 food_ids 空数组", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 102);
    expect(dlg.find(".foods").exists()).toBe(false);

    await dlg.find(".save-btn").trigger("click");
    await flushPromises();

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "update_log");
    expect(call).toBeDefined();
    expect((call![1] as { input: { food_ids: number[] } }).input.food_ids).toEqual([]);
  });

  it("编辑失败：弹窗保持并展示原因，不误刷列表", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);
    invokeMock.mockClear();

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "update_log") {
        throw "食物「面包虫」已停用，不能新选";
      }
      return baseImpl(cmd);
    });
    await dlg.find(".save-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".edit-dialog .form-error").text()).toContain("已停用");
    expect(wrapper.find(".edit-dialog").exists()).toBe(true);
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs").length).toBe(0);
    expect(wrapper.emitted("changed")).toBeFalsy();
  });

  it("删除两段确认：第一次只进入确认态，第二次才发 delete_log 并刷新（验收 3 的入口）", async () => {
    const wrapper = await mountPage();
    const delBtn = () => wrapper.find('.log-row[data-log-id="101"] .delete-btn');

    invokeMock.mockClear();
    await delBtn().trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "delete_log")).toBe(false);
    expect(delBtn().text()).toContain("确认删除");

    await delBtn().trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("delete_log", { id: 101 });
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(true);
    expect(wrapper.emitted("changed")).toBeTruthy();
  });

  it("重置筛选回全量查询", async () => {
    const wrapper = await mountPage();
    await wrapper.find(".f-colony").setValue("2");
    await flushPromises();

    invokeMock.mockClear();
    await wrapper.find(".reset-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("list_logs", {
      filter: {
        location_id: null,
        colony_id: null,
        action_id: null,
        start: null,
        end: null,
        note_keyword: null,
        limit: 50,
        offset: 0,
      },
    });
    expect((wrapper.find(".f-colony").element as HTMLSelectElement).value).toBe("");
  });

  it("撤食流水切片（票 03）：筛选下拉含撤食，选中即按该操作查询（通用字典，无白名单）", async () => {
    const wrapper = await mountPage();
    const actionOpts = wrapper.findAll(".f-action option").map((o) => o.text());
    expect(actionOpts, "停用项不进筛选入口").toEqual(["全部", "喂食", "活动区换水", "撤食"]);

    invokeMock.mockClear();
    await wrapper.find(".f-action").setValue("4");
    await flushPromises();
    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_logs");
    expect(calls.length).toBe(1);
    expect((calls[0][1] as { filter: { action_id: number } }).filter.action_id).toBe(4);
  });

  it("撤食行照常编辑/删除（票 03）：无食物区、food_ids 空数组提交、两段确认删除", async () => {
    currentPage = {
      total: 1,
      rows: [
        {
          id: 201,
          colony_id: 1,
          colony_name: "大头一号",
          location_name: "家",
          action_id: 4,
          action_name: "撤食",
          occurred_at: "2026-09-18 09:00:00",
          created_at: "2026-09-18 09:00:00",
          note: "收走面包虫",
          food_ids: [],
          food_names: [],
        },
      ],
    };
    const wrapper = await mountPage();

    // 行渲染：操作列显示撤食、食物列占位、编辑/删除按钮照常
    const row = wrapper.find('.log-row[data-log-id="201"]');
    expect(row.find(".c-action").text()).toBe("撤食");
    expect(row.find(".c-foods").text()).toBe("—");

    // 编辑：弹窗操作选中撤食、非喂食无食物区，提交 update_log（food_ids 空数组）
    const dlg = await openEdit(wrapper, 201);
    expect((dlg.find(".action-select").element as HTMLSelectElement).value).toBe("4");
    expect(dlg.find(".foods").exists()).toBe(false);
    await dlg.find(".save-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("update_log", {
      id: 201,
      input: { occurred_at: "2026-09-18T09:00", action_id: 4, food_ids: [], note: "收走面包虫" },
    });

    // 删除：两段确认后 delete_log
    const delBtn = () => wrapper.find('.log-row[data-log-id="201"] .delete-btn');
    await delBtn().trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "delete_log")).toBe(false);
    await delBtn().trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("delete_log", { id: 201 });
    expect(wrapper.emitted("changed")).toBeTruthy();
  });
});
