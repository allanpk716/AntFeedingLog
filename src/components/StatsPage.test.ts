import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import StatsPage from "./StatsPage.vue";
import type { Colony, StatsPayload } from "../types";
import { addDays, todayIso } from "../lib/dates";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 LogListPage.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

// happy-dom 无 canvas：mock 掉 echarts，只断言 setOption 收到配置
const { echartsSetOption, echartsDispose } = vi.hoisted(() => ({
  echartsSetOption: vi.fn(),
  echartsDispose: vi.fn(),
}));
vi.mock("echarts", () => ({
  init: vi.fn(() => ({ setOption: echartsSetOption, dispose: echartsDispose, resize: vi.fn() })),
}));

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

function makePayload(overrides: Partial<StatsPayload> = {}): StatsPayload {
  return {
    range_start: "2026-03-23",
    range_end: todayIso(),
    range_days: 180,
    daily: [
      { date: "2026-09-10", count: 2 },
      { date: "2026-09-12", count: 1 },
    ],
    daily_detail: [
      {
        date: "2026-09-10",
        entries: [
          { action_name: "喂食", food_names: ["种子", "干虾仁"] },
          { action_name: "巢穴保湿", food_names: [] },
        ],
      },
    ],
    food_share: [
      { food_name: "种子", occurrences: 2 },
      { food_name: "干虾仁", occurrences: 1 },
    ],
    weekly: [{ week_start: "2026-09-14", count: 3 }],
    intervals: [
      {
        action_id: 1,
        name: "喂食",
        kind: "reminding",
        suggested_interval_days: 3,
        sample_count: 2,
        avg_days: 6,
        min_days: 5,
        max_days: 7,
      },
      {
        action_id: 2,
        name: "活动区换水",
        kind: "log_only",
        suggested_interval_days: null,
        sample_count: 0,
        avg_days: null,
        min_days: null,
        max_days: null,
      },
    ],
    ...overrides,
  };
}

let currentStats: StatsPayload | null = makePayload();

function baseMock() {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_colonies":
        return colonies;
      case "get_stats":
        return currentStats;
      default:
        return null;
    }
  });
}

async function mountPage() {
  const wrapper = mount(StatsPage);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  echartsSetOption.mockClear();
  echartsDispose.mockClear();
  currentStats = makePayload();
  baseMock();
});

describe("统计页（票 07）", () => {
  it("挂载后拉窝清单并发起默认统计：全部窝 · 近 6 个月（窗口含今天 180 天）", async () => {
    const wrapper = await mountPage();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    expect(invokeMock).toHaveBeenCalledWith("get_stats", {
      colonyId: null,
      startDate: addDays(todayIso(), -179),
      endDate: todayIso(),
    });
    // 筛选下拉：全部 + 每窝
    const options = wrapper.findAll(".colony-select option");
    expect(options.map((o) => o.text())).toEqual(["全部", "大头一号", "针毛一号"]);
  });

  it("切换窝与时间范围后重新统计（验收 5：筛选联动全部图表）", async () => {
    const wrapper = await mountPage();
    invokeMock.mockClear();

    await wrapper.find(".colony-select").setValue("2");
    await wrapper.find(".range-select").setValue("3m");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("get_stats", {
      colonyId: 2,
      startDate: addDays(todayIso(), -89),
      endDate: todayIso(),
    });
  });

  it("「全部」下界 = min(最早开始饲养日, 最早记录日)：早于饲养日的补录不消失（票 07 停靠①）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_colonies":
          return colonies; // start_date 最早 2026-01-20
        case "get_stats":
          return currentStats;
        case "earliest_log_date":
          return "2025-12-01";
        default:
          return null;
      }
    });
    const wrapper = await mountPage();
    await wrapper.find(".range-select").setValue("all");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("earliest_log_date");
    expect(invokeMock).toHaveBeenCalledWith("get_stats", {
      colonyId: null,
      startDate: "2025-12-01",
      endDate: todayIso(),
    });
  });

  it("频率口径标注可见：平均每天 = 总数 ÷ 自然日天数（不扣冬眠）（验收 6）", async () => {
    const wrapper = await mountPage();
    // 样例：总数 3、range_days 180 → 0.0；标注含天数与「不扣冬眠」
    expect(wrapper.text()).toContain("不扣冬眠");
    expect(wrapper.text()).toContain("180");
    expect(wrapper.text()).toContain("0.0");
  });

  it("喂食构成：归一化合计 100%、图例带百分比与出现次数（验收 1）", async () => {
    const wrapper = await mountPage();
    const legend = wrapper.find(".dlegend");
    expect(legend.text()).toContain("种子");
    expect(legend.text()).toContain("67%");
    expect(legend.text()).toContain("2 次");
    expect(legend.text()).toContain("干虾仁");
    expect(legend.text()).toContain("33%");
    expect(legend.text()).toContain("1 次");
    // 3 份食物 2:1 → 66.7/33.3 → 67/33，合计恰 100
    const percents = legend.findAll(".pct").map((n) => Number(n.text().replace("%", "")));
    expect(percents.reduce((s, p) => s + p, 0)).toBe(100);
  });

  it("间隔条：提醒类画建议刻度竖线，登记类标「仅登记」，无样本标「记录不足」", async () => {
    const wrapper = await mountPage();
    const rows = wrapper.findAll(".irow");
    expect(rows.length).toBe(2);

    const feed = rows[0];
    expect(feed.find(".imark").exists()).toBe(true);
    expect(feed.text()).toContain("建议 3 天");
    expect(feed.text()).toContain("平均");

    const water = rows[1];
    expect(water.find(".imark").exists()).toBe(false);
    expect(water.text()).toContain("仅登记");
    expect(water.text()).toContain("记录不足");
  });

  it("空数据/单天范围不崩溃：零计数也能渲染图表配置，并显示暂无提示（验收 4）", async () => {
    currentStats = makePayload({
      range_days: 1,
      daily: [{ date: todayIso(), count: 0 }],
      daily_detail: [],
      food_share: [],
      weekly: [],
      intervals: [],
    });
    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("暂无记录");
    expect(wrapper.find(".dlegend .empty").exists()).toBe(true);
    expect(echartsSetOption).toHaveBeenCalled();
    const rows = wrapper.findAll(".irow");
    expect(rows.length).toBe(0); // 空 intervals 不渲染间隔行
  });
});
