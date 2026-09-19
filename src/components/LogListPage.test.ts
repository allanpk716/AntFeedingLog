import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import LogListPage from "./LogListPage.vue";
import type { CareActionItem, Colony, FoodItem, LogPage, LogRow } from "../types";

// 不依赖 Tauri 运行时：mock 掉 IPC
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const colonies: Colony[] = [
  {
    id: 1,
    name: "大头一号",
    species: null,
    location_id: null,
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
    location_id: null,
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
];

const foods: FoodItem[] = [
  { id: 1, name: "种子", enabled: true, sort: 1, suggested_interval_days: 3, referenced: false, is_preset: true },
  { id: 2, name: "干虾仁", enabled: false, sort: 2, suggested_interval_days: 7, referenced: true, is_preset: true },
  { id: 3, name: "面包虫", enabled: true, sort: 3, suggested_interval_days: 7, referenced: false, is_preset: true },
  { id: 4, name: "糖水", enabled: false, sort: 4, suggested_interval_days: null, referenced: false, is_preset: false },
];

/** 行 101：喂食、引用了停用食物「干虾仁」；行 102：换水（非喂食）；行 103 留给「加载更多」 */
const rows: LogRow[] = [
  {
    id: 101,
    colony_id: 1,
    colony_name: "大头一号",
    location_name: null,
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
    location_name: null,
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

function baseMock() {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_colonies":
        return colonies;
      case "list_actions":
        return actions;
      case "list_foods":
        return foods;
      case "list_logs":
        return currentPage;
      default:
        return null;
    }
  });
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

beforeEach(() => {
  invokeMock.mockReset();
  currentPage = { total: 3, rows };
  baseMock();
});

describe("记录列表页（票 08）", () => {
  it("挂载拉字典并发起默认查询：表格渲染时间/窝/操作/食物/备注，total 可见", async () => {
    const wrapper = await mountPage();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    expect(invokeMock).toHaveBeenCalledWith("list_actions");
    expect(invokeMock).toHaveBeenCalledWith("list_foods");
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

    const rows = wrapper.findAll(".log-row");
    expect(rows.length).toBe(2);
    expect(rows[0].find(".c-time").text()).toBe("2026-09-17 21:00");
    expect(rows[0].find(".c-colony").text()).toBe("大头一号");
    expect(rows[0].find(".c-action").text()).toBe("喂食");
    expect(rows[0].find(".c-foods").text()).toContain("干虾仁");
    expect(rows[0].find(".c-note").text()).toBe("加餐");
    // 空备注显示占位
    expect(rows[1].find(".c-note").text()).toBe("—");
    expect(wrapper.find(".total-note").text()).toContain("3");
  });

  it("筛选组合生效：点查询以组合好的 filter 重新发起（验收 1）", async () => {
    const wrapper = await mountPage();
    invokeMock.mockClear();

    await wrapper.find(".f-colony").setValue("1");
    await wrapper.find(".f-action").setValue("1");
    await wrapper.find(".f-start").setValue("2026-09-01");
    await wrapper.find(".f-end").setValue("2026-09-18");
    await wrapper.find(".f-keyword").setValue("  面包虫  ");
    await wrapper.find(".apply-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("list_logs", {
      filter: {
        location_id: null,
        colony_id: 1,
        action_id: 1,
        start: "2026-09-01",
        end: "2026-09-18",
        note_keyword: "面包虫",
        limit: 50,
        offset: 0,
      },
    });
  });

  it("时间范围倒置前端先拦：不发起查询并提示", async () => {
    const wrapper = await mountPage();
    invokeMock.mockClear();

    await wrapper.find(".f-start").setValue("2026-09-10");
    await wrapper.find(".f-end").setValue("2026-09-01");
    await wrapper.find(".apply-btn").trigger("click");
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

  it("编辑弹窗预填当前值；停用项标「已停用」：原引用可选、无关停用禁选（规则 10）", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);

    expect((dlg.find(".time-input").element as HTMLInputElement).value).toBe("2026-09-17T21:00");
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
    expect(byName("面包虫").attributes("disabled")).toBeUndefined();  });

  it("编辑喂食记录更换食物提交 update_log（验收 2），成功后关窗、重查列表并通知外层重算", async () => {
    const wrapper = await mountPage();
    const dlg = await openEdit(wrapper, 101);

    await dlg.find(".time-input").setValue("2026-09-10T08:30");
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
      if (cmd === "list_logs") {
        return currentPage;
      }
      return null;
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
    await wrapper.find(".apply-btn").trigger("click");
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
});
