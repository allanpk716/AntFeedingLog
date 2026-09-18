import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import App from "./App.vue";
import type { CareActionItem, Colony, ColonyAction, FoodItem, LocationItem, RecentLog } from "./types";
import { addDays, todayIso } from "./lib/dates";

// 不依赖 Tauri 运行时：mock 掉 IPC，按命令名回放数据
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const locations: LocationItem[] = [
  { id: 1, name: "家", enabled: true, sort: 1 },
  { id: 2, name: "公司", enabled: true, sort: 2 },
];

const foods: FoodItem[] = [
  { id: 1, name: "种子", enabled: true, sort: 1, referenced: false },
  { id: 2, name: "干虾仁", enabled: true, sort: 2, referenced: false },
  { id: 3, name: "面包虫", enabled: true, sort: 3, referenced: false },
  { id: 4, name: "蚕蛹", enabled: false, sort: 4, referenced: false },
];

const actions: CareActionItem[] = [
  { id: 1, name: "喂食", icon: null, kind: "reminding", is_feeding: true, suggested_interval_days: 3, enabled: true, sort: 1, referenced: false },
  { id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 2, referenced: true },
  { id: 3, name: "巢穴保湿", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 3, referenced: false },
  { id: 4, name: "垃圾清理", icon: null, kind: "reminding", is_feeding: false, suggested_interval_days: 7, enabled: true, sort: 4, referenced: false },
];

const colonies: Colony[] = [
  {
    id: 1,
    name: "大头一号",
    species: "大头收获蚁",
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
    species: "针毛收获蚁",
    location_id: 2,
    start_date: "2026-06-14",
    status: "active",
    days_raised: 96,
    actions: [],
    recent: [],
    hibernation: null,
  },
  {
    id: 3,
    name: "大头二号",
    species: "大头收获蚁",
    location_id: 2,
    start_date: "2026-02-10",
    status: "hibernating",
    days_raised: 220,
    actions: [],
    recent: [],
    hibernation: null,
  },
  {
    id: 4,
    name: "老窝",
    species: null,
    location_id: null,
    start_date: "2025-01-01",
    status: "ended",
    days_raised: 626,
    actions: [],
    recent: [],
    hibernation: null,
  },
];

/** list_colonies 返回的数据源，测试里可整体替换（模拟后端重算后的新数据）。 */
let currentColonies: Colony[] = colonies;

function baseMock() {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_colonies":
        return currentColonies;
      case "list_locations":
        return locations;
      case "list_foods":
        return foods;
      case "list_actions":
        return actions;
      default:
        return null;
    }
  });
}

async function mountApp() {
  const wrapper = mount(App);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  currentColonies = colonies;
  baseMock();
});

describe("首页卡片墙", () => {
  it("按地点清单顺序分组，卡片含名字/物种徽章/状态徽章/饲养天数大数字", async () => {
    const wrapper = await mountApp();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    expect(invokeMock).toHaveBeenCalledWith("list_locations");

    const titles = wrapper.findAll(".group-title").map((t) => t.text());
    expect(titles).toEqual(["家", "公司"]);

    const home = wrapper.find('.card[data-colony-id="1"]');
    expect(home.find(".cname").text()).toBe("大头一号");
    expect(home.find(".chip.sp").text()).toBe("大头收获蚁");
    expect(home.find(".chip.st").text()).toContain("活跃");
    expect(home.find(".daysbox .n").text()).toBe("241");
    expect(home.text()).toContain("已饲养 / 天");
    expect(home.text()).toContain("开始饲养 2026-01-20");

    const corpCards = wrapper.findAll(".group")[1].findAll(".card");
    expect(corpCards.map((c) => c.find(".cname").text())).toEqual(["针毛一号", "大头二号"]);
  });

  it("冬眠中的窝显示冬眠状态徽章", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find('.card[data-colony-id="3"] .chip.st').text()).toContain("冬眠");
  });

  it("已结束的窝默认折叠，展开后可见，且不占分组", async () => {
    const wrapper = await mountApp();

    const section = wrapper.find(".ended-section");
    expect(section.exists()).toBe(true);
    expect((section.element as HTMLElement).style.display).toBe("none");

    // 分组卡片里不出现已结束的窝
    expect(wrapper.find(".group").text()).not.toContain("老窝");

    await wrapper.find(".ended-toggle").trigger("click");
    const opened = wrapper.find(".ended-section");
    expect((opened.element as HTMLElement).style.display).not.toBe("none");
    const endedCard = opened.find('.card[data-colony-id="4"]');
    expect(endedCard.exists()).toBe(true);
    expect(endedCard.find(".chip.sp").exists()).toBe(false); // 无物种不渲染徽章
    expect(endedCard.find(".chip.st").text()).toContain("已结束");
  });

  it("没有任何窝时显示「暂无窝」并保留新建入口", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_colonies" ? [] : cmd === "list_locations" ? locations : null,
    );
    const wrapper = await mountApp();
    expect(wrapper.find(".empty").text()).toContain("暂无窝");
    expect(wrapper.find(".new-colony").exists()).toBe(true);
  });
});

describe("新建窝", () => {
  it("填表提交调用 create_colony（trim 后的 snake_case 入参），成功后关弹窗并刷新", async () => {
    const wrapper = await mountApp();
    await wrapper.find(".new-colony").trigger("click");

    const dialog = wrapper.find(".dialog");
    expect(dialog.exists()).toBe(true);
    expect((dialog.find(".name-input").element as HTMLInputElement).value).toBe("");

    await dialog.find(".name-input").setValue("  新窝一号  ");
    await dialog.find(".species-input").setValue("针毛收获蚁");
    await dialog.find(".location-select").setValue("1");
    await dialog.find(".date-input").setValue("2026-09-18");

    invokeMock.mockClear();
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("create_colony", {
      input: {
        name: "新窝一号",
        species: "针毛收获蚁",
        location_id: 1,
        start_date: "2026-09-18",
        status: "active",
      },
    });
    expect(wrapper.find(".dialog").exists()).toBe(false);
    const refreshCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_colonies");
    expect(refreshCalls.length).toBeGreaterThanOrEqual(1);
  });

  it("重名（含首尾空格差异）被前端拦截，不发起 create_colony", async () => {
    const wrapper = await mountApp();
    await wrapper.find(".new-colony").trigger("click");

    const dialog = wrapper.find(".dialog");
    await dialog.find(".name-input").setValue("  大头一号 ");
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(dialog.find(".form-error").text()).toContain("已存在");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "create_colony")).toBe(false);
    expect(wrapper.find(".dialog").exists()).toBe(true);
  });
});

describe("编辑窝", () => {
  it("预填当前值，修改后调用 update_colony（带 id）", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .edit-btn').trigger("click");

    const dialog = wrapper.find(".dialog");
    expect((dialog.find(".name-input").element as HTMLInputElement).value).toBe("大头一号");
    expect((dialog.find(".species-input").element as HTMLInputElement).value).toBe("大头收获蚁");
    expect((dialog.find(".location-select").element as HTMLSelectElement).value).toBe("1");
    expect((dialog.find(".date-input").element as HTMLInputElement).value).toBe("2026-01-20");

    await dialog.find(".name-input").setValue("大头一号B");
    await dialog.find(".status-select").setValue("hibernating");

    invokeMock.mockClear();
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("update_colony", {
      id: 1,
      input: {
        name: "大头一号B",
        species: "大头收获蚁",
        location_id: 1,
        start_date: "2026-01-20",
        status: "hibernating",
      },
    });
  });

  it("「置为已结束」调用 archive_colony", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .edit-btn').trigger("click");

    invokeMock.mockClear();
    await wrapper.find(".dialog .archive-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("archive_colony", { id: 1 });
    expect(wrapper.find(".dialog").exists()).toBe(false);
  });

  it("有记录的窝删除被后端拒绝时，弹窗内展示原因且窝保留", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .edit-btn').trigger("click");

    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "delete_colony":
          throw "该窝已有 3 条记录，不能删除；可改为置为「已结束」";
        case "list_colonies":
          return colonies;
        case "list_locations":
          return locations;
        default:
          return null;
      }
    });

    await wrapper.find(".dialog .delete-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("delete_colony", { id: 1 });
    expect(wrapper.find(".dialog .form-error").text()).toContain("不能删除");
    expect(wrapper.find('.card[data-colony-id="1"]').exists()).toBe(true);
  });

  it("无记录的窝删除成功，关弹窗", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="2"] .edit-btn').trigger("click");

    invokeMock.mockClear();
    await wrapper.find(".dialog .delete-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("delete_colony", { id: 2 });
    expect(wrapper.find(".dialog").exists()).toBe(false);
  });
});

describe("设置 · 字典管理（票 04）", () => {
  /** 打开设置弹窗并等它拉完字典。 */
  async function openSettings(wrapper: Awaited<ReturnType<typeof mountApp>>) {
    await wrapper.find(".settings-btn").trigger("click");
    await flushPromises();
    return wrapper.find(".settings-dialog");
  }

  // ── 地点 tab（复用 LocationManagerPanel）──

  it("地点 tab：列出全部地点，支持上下移调顺序，保存时按新顺序逐行 save_location，弹窗不关", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);
    await dlg.find(".tab-locations").trigger("click");

    const rows = dlg.findAll(".loc-row");
    expect(rows.length).toBe(2);
    expect((rows[0].find(".loc-name-input").element as HTMLInputElement).value).toBe("家");

    // 把「公司」上移到第一位
    await rows[1].find(".move-up").trigger("click");

    invokeMock.mockClear();
    await dlg.find(".save-locations").trigger("click");
    await flushPromises();

    const saveCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "save_location");
    expect(saveCalls).toEqual([
      ["save_location", { input: { id: 2, name: "公司", sort: 0 } }],
      ["save_location", { input: { id: 1, name: "家", sort: 1 } }],
    ]);
    // 设置弹窗保存后保持打开（changed → 外层静默刷新首页）
    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("地点 tab：可新增地点（新行 id 为空），保存时一并提交", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);
    await dlg.find(".tab-locations").trigger("click");

    await dlg.find(".loc-add-input").setValue("阳台");
    await dlg.find(".loc-add-btn").trigger("click");
    expect(dlg.findAll(".loc-row").length).toBe(3);

    invokeMock.mockClear();
    await dlg.find(".save-locations").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_location", {
      input: { id: null, name: "阳台", sort: 2 },
    });
  });

  it("地点 tab：行级操作不关弹窗，删除被引用展示原因，停用后可再启用", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);
    await dlg.find(".tab-locations").trigger("click");

    // 未保存的改名（行内编辑）
    await dlg.find(".loc-row .loc-name-input").setValue("老家");

    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "erase_location":
          throw "该地点仍被 1 个窝使用，不能删除；可改为停用";
        case "list_colonies":
          return colonies;
        case "list_locations":
          return locations;
        case "list_foods":
          return foods;
        case "list_actions":
          return actions;
        default:
          return null;
      }
    });

    const rows = dlg.findAll(".loc-row");
    await rows[0].find(".loc-erase-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("erase_location", { id: 1 });
    expect(dlg.find(".loc-error").text()).toContain("停用");
    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(dlg.findAll(".loc-row").length).toBe(2);

    await rows[0].find(".loc-deactivate-btn").trigger("click");
    await flushPromises();

    // 票 04：停用改走双向 set_location_enabled
    expect(invokeMock).toHaveBeenCalledWith("set_location_enabled", { id: 1, enabled: false });
    // 弹窗仍开着；行内未保存的改名保留；停用态行内可见
    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(
      (dlg.find(".loc-row .loc-name-input").element as HTMLInputElement).value,
    ).toBe("老家");
    expect(dlg.find(".loc-row .loc-disabled-chip").exists()).toBe(true);

    // 票 04 补的恢复通道：停用的地点可再启用
    invokeMock.mockClear();
    await dlg.find(".loc-row .loc-activate-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_location_enabled", { id: 1, enabled: true });
  });

  // ── 操作 tab ──

  it("操作 tab：列出全部操作，被引用的行删除禁用并提示只能停用", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);

    const rows = dlg.findAll(".dict-row");
    expect(
      rows.map((r) => (r.find(".name-input").element as HTMLInputElement).value),
    ).toEqual(["喂食", "活动区换水", "巢穴保湿", "垃圾清理"]);

    // 活动区换水被历史记录引用 → 删除禁用（规则 10）
    const water = rows[1];
    expect(water.find(".erase-btn").attributes("disabled")).toBeDefined();
    expect(water.find(".erase-btn").attributes("title")).toContain("只能停用");
    // 未被引用的可删
    expect(rows[2].find(".erase-btn").attributes("disabled")).toBeUndefined();
  });

  it("操作 tab：登记类切提醒 + 填建议间隔，保存发出 save_action 并刷新首页（验收 4 的链路）", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);

    const water = dlg.findAll(".dict-row")[1];
    expect((water.find(".kind-select").element as HTMLSelectElement).value).toBe("log_only");
    // 登记类不显示间隔输入
    expect(water.find(".interval-input").exists()).toBe(false);

    await water.find(".kind-select").setValue("reminding");
    const interval = water.find(".interval-input");
    expect(interval.exists()).toBe(true);
    await interval.setValue("3");

    invokeMock.mockClear();
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_action", {
      input: {
        id: 2,
        name: "活动区换水",
        kind: "reminding",
        is_feeding: false,
        suggested_interval_days: 3,
        sort: 1,
      },
    });
    // changed → 外层刷新首页数据（卡片红/灰数据驱动重算，验收 4）
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("操作 tab：停用即时生效（set_action_enabled + changed 刷新），弹窗不关", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);

    invokeMock.mockClear();
    const hydrate = dlg.findAll(".dict-row")[2];
    await hydrate.find(".row-btn").trigger("click"); // 启用行的第一个行级按钮是「停用」
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_action_enabled", { id: 3, enabled: false });
    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("操作 tab：可新增操作（默认提醒类、建议间隔 7），保存一并提交", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);

    await dlg.find(".add-input").setValue("糖水");
    await dlg.find(".add-btn").trigger("click");
    expect(dlg.findAll(".dict-row").length).toBe(5);

    invokeMock.mockClear();
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_action", {
      input: {
        id: null,
        name: "糖水",
        kind: "reminding",
        is_feeding: false,
        suggested_interval_days: 7,
        sort: 4,
      },
    });
  });

  // ── 食物 tab ──

  it("食物 tab：改名保存走 save_food；停用即时生效（验收 1 的入口）；停用行置灰", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);
    await dlg.find(".tab-foods").trigger("click");

    const rows = dlg.findAll(".dict-row");
    expect(
      rows.map((r) => (r.find(".name-input").element as HTMLInputElement).value),
    ).toEqual(["种子", "干虾仁", "面包虫", "蚕蛹"]);
    // 停用的「蚕蛹」整行置灰 + 行级第一按钮是「启用」
    expect(rows[3].classes()).toContain("row-disabled");
    expect(rows[3].find(".row-btn").text()).toBe("启用");

    await rows[0].find(".name-input").setValue("瓜子");
    invokeMock.mockClear();
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("save_food", {
      input: { id: 1, name: "瓜子", sort: 0 },
    });

    invokeMock.mockClear();
    await dlg.findAll(".dict-row")[1].find(".row-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_food_enabled", { id: 2, enabled: false });
  });
});

describe("卡片操作块与一键记账（票 03）", () => {
  const feedOverdue: ColonyAction = {
    action_id: 1,
    name: "喂食",
    icon: null,
    kind: "reminding",
    is_feeding: true,
    suggested_interval_days: 3,
    days_since_last: 4,
    overdue: true,
  };
  const waterReg: ColonyAction = {
    action_id: 2,
    name: "活动区换水",
    icon: null,
    kind: "log_only",
    is_feeding: false,
    suggested_interval_days: null,
    days_since_last: 2,
    overdue: false,
  };
  const hydrateNever: ColonyAction = {
    action_id: 3,
    name: "巢穴保湿",
    icon: null,
    kind: "log_only",
    is_feeding: false,
    suggested_interval_days: null,
    days_since_last: null,
    overdue: false,
  };
  const trashToday: ColonyAction = {
    action_id: 4,
    name: "垃圾清理",
    icon: null,
    kind: "reminding",
    is_feeding: false,
    suggested_interval_days: 7,
    days_since_last: 0,
    overdue: false,
  };
  // 名字不含「喂食」的喂食类操作：弹窗触发与标题只认 is_feeding 位（R1 解耦）
  const feedCustom: ColonyAction = {
    action_id: 9,
    name: "投喂",
    icon: null,
    kind: "reminding",
    is_feeding: true,
    suggested_interval_days: 3,
    days_since_last: 9,
    overdue: true,
  };

  const recent: RecentLog[] = [
    { happened_at: "2026-09-17 20:00:00", action_name: "喂食", food_names: ["种子"] },
  ];

  function colony1With(actions: ColonyAction[], withRecent: RecentLog[] = []): void {
    currentColonies = colonies.map((c) =>
      c.id === 1 ? { ...c, actions, recent: withRecent } : c,
    );
  }

  it("每个启用操作动态渲染一块：超期红/登记灰带角标/今天绿/未记录中性，底部显示最近摘要", async () => {
    colony1With([feedOverdue, waterReg, hydrateNever, trashToday], recent);
    const wrapper = await mountApp();

    const card = wrapper.find('.card[data-colony-id="1"]');
    const tiles = card.findAll(".tile");
    expect(tiles.length).toBe(4); // 操作块随字典启用项渲染

    // 超期：喂食 4 天 > 建议 3 → 红（验收 4）
    const feed = card.find('.tile[data-action-id="1"]');
    expect(feed.classes()).toContain("bad");
    expect(feed.find(".pill").text()).toBe("⚠ 超期 1 天");

    // 登记类：中性灰 +「仅登记」角标，永不红（验收 5）
    const water = card.find('.tile[data-action-id="2"]');
    expect(water.classes()).toContain("reg");
    expect(water.classes()).not.toContain("bad");
    expect(water.find(".t-tag").text()).toBe("仅登记");
    expect(water.find(".pill").text()).toBe("距上次 2 天");

    // 从未记录：中性
    expect(card.find('.tile[data-action-id="3"]').find(".pill").text()).toBe("尚未记录");

    // 今天已记录：绿（验收 1 的展示态）
    const trash = card.find('.tile[data-action-id="4"]');
    expect(trash.classes()).toContain("ok");
    expect(trash.find(".pill").text()).toBe("今天 · 已记录");

    // 最近记录摘要
    expect(card.find(".recent").text()).toBe("最近：09-17 喂食（种子）");
  });

  it("非喂食一点即记：log_care 带默认现在时间、无食物，成功后数据驱动刷新且块变「今天 · 已记录」", async () => {
    colony1With([waterReg]);
    const wrapper = await mountApp();

    invokeMock.mockClear();
    const click = wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
    // 记账成功后外层 refresh，后端算出距上次 0
    colony1With([{ ...waterReg, days_since_last: 0 }]);
    await click;
    await flushPromises();

    const logCall = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(logCall).toBeDefined();
    expect(logCall![0]).toBe("log_care");
    const input = logCall![1] as { input: { colony_id: number; action_id: number; happened_at: string; note: string | null; food_ids: number[] } };
    expect(input.input.colony_id).toBe(1);
    expect(input.input.action_id).toBe(2);
    expect(input.input.happened_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
    expect(input.input.note).toBeNull();
    expect(input.input.food_ids).toEqual([]);

    const refreshCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_colonies");
    expect(refreshCalls.length).toBeGreaterThanOrEqual(1);
    expect(
      wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"] .pill').text(),
    ).toBe("今天 · 已记录");
  });

  it("点喂食类操作弹食物多选弹窗（只认 is_feeding 位）：只列启用食物，勾两种 + 补录时间 + 备注，确认生成一条带两食物的记录", async () => {
    // 操作名「投喂」不含「喂食」：弹窗触发与标题跟随 is_feeding/名字，不受字典改名影响
    colony1With([feedCustom]);
    const wrapper = await mountApp();

    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="9"]').trigger("click");

    const dialog = wrapper.find(".feed-dialog");
    expect(dialog.exists()).toBe(true);
    expect(dialog.find("h3").text()).toBe("记录投喂 · 大头一号");
    expect(invokeMock).toHaveBeenCalledWith("list_foods");

    // 停用食物不出现在新建记录入口（规则 10）
    const chips = dialog.findAll(".food");
    expect(chips.map((c) => c.text())).toEqual(["种子", "干虾仁", "面包虫"]);

    await chips[1].trigger("click"); // 干虾仁
    await chips[2].trigger("click"); // 面包虫
    expect(chips[1].classes()).toContain("selected");
    expect(chips[0].classes()).not.toContain("selected");

    // 补录昨天时间（验收 3：距上次按发生时间算，Rust 侧覆盖计算）
    await dialog.find(".time-input").setValue("2026-09-17T21:00");
    await dialog.find(".note-input").setValue("  加餐  ");

    invokeMock.mockClear();
    await dialog.find(".record-btn").trigger("click");
    await flushPromises();

    const logCall = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(logCall).toBeDefined();
    expect(logCall![1]).toEqual({
      input: {
        colony_id: 1,
        action_id: 9,
        happened_at: "2026-09-17T21:00",
        note: "加餐",
        food_ids: [2, 3],
      },
    });
    // saved → 关窗 + 外层刷新
    expect(wrapper.find(".feed-dialog").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("喂食弹窗未选食物不允许提交，弹窗保持", async () => {
    colony1With([feedOverdue]);
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="1"]').trigger("click");

    invokeMock.mockClear();
    await wrapper.find(".feed-dialog .record-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".feed-dialog .form-error").text()).toContain("先选至少一种食物");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "log_care")).toBe(false);
    expect(wrapper.find(".feed-dialog").exists()).toBe(true);
  });

  it("取消关闭喂食弹窗且不记账", async () => {
    colony1With([feedOverdue]);
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="1"]').trigger("click");

    invokeMock.mockClear();
    await wrapper.find(".feed-dialog .cancel-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".feed-dialog").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "log_care")).toBe(false);
  });

  it("冬眠窝的操作块全部静音态：不红、文案带（静音）", async () => {
    currentColonies = colonies.map((c) =>
      c.id === 3 ? { ...c, actions: [feedOverdue] } : c,
    );
    const wrapper = await mountApp();

    const tile = wrapper.find('.card[data-colony-id="3"] .tile[data-action-id="1"]');
    expect(tile.classes()).toContain("mute");
    expect(tile.classes()).not.toContain("bad");
    expect(tile.find(".pill").text()).toBe("距上次 4 天（静音）");
  });

  it("一键记账失败：卡片上展示原因，不误刷数据", async () => {
    colony1With([waterReg]);
    const wrapper = await mountApp();
    invokeMock.mockClear();

    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "log_care":
          throw "操作「活动区换水」已停用，不能新记";
        case "list_colonies":
          return currentColonies;
        case "list_locations":
          return locations;
        default:
          return null;
      }
    });

    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
    await flushPromises();

    expect(wrapper.find('.card[data-colony-id="1"] .tile-error').text()).toContain("不能新记");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(false);
  });
});

describe("冬眠管理（票 05）", () => {
  /** 把 3 号窝（冬眠中）换成带开放段数据的版本。 */
  function colony3With(hibernation: { id: number; start_date: string; expected_end_date: string } | null): void {
    currentColonies = colonies.map((c) => (c.id === 3 ? { ...c, hibernation } : c));
  }

  it("冬眠卡灰化 + 横幅：入眠日/预计出眠/剩余天数；未到临近窗口不显角标", async () => {
    // 预计出眠远在将来（2099）：剩余天数很多 → 无「临近出眠」角标（与真实今天无关，判定稳定）
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "2099-01-01" });
    const wrapper = await mountApp();

    const card = wrapper.find('.card[data-colony-id="3"]');
    expect(card.classes()).toContain("hib");
    const banner = card.find(".banner");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("08-20 入眠");
    expect(banner.text()).toContain("预计出眠 01-01");
    expect(banner.text()).toMatch(/还有 \d+ 天/);
    expect(banner.find(".chip.wake").exists()).toBe(false);
  });

  it("预计出眠已过仍冬眠：横幅提示已过 N 天并显示「临近出眠」角标", async () => {
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "1999-01-01" });
    const wrapper = await mountApp();

    const banner = wrapper.find('.card[data-colony-id="3"] .banner');
    expect(banner.text()).toContain("预计日已过");
    expect(banner.find(".chip.wake").text()).toBe("临近出眠");
  });

  it("活跃卡有「开始冬眠」入口：默认开始=今天、预计结束=开始+120 天，可改后提交 start_hibernation 并刷新", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .hib-btn').trigger("click");

    const dialog = wrapper.find(".hibernation-dialog");
    expect(dialog.exists()).toBe(true);
    const start = (dialog.find(".start-input").element as HTMLInputElement).value;
    expect(start).toBe(todayIso());
    expect((dialog.find(".end-input").element as HTMLInputElement).value).toBe(addDays(start, 120));

    await dialog.find(".start-input").setValue("2026-12-01");
    // 手动改预计结束（选完开始日自动预填、但用户可改）
    await dialog.find(".end-input").setValue("2027-04-15");

    invokeMock.mockClear();
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("start_hibernation", {
      colonyId: 1,
      startDate: "2026-12-01",
      expectedEndDate: "2027-04-15",
    });
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("改开始日且未手动改过预计结束时，预计结束自动跟随重填（+120 天）", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .hib-btn').trigger("click");

    const dialog = wrapper.find(".hibernation-dialog");
    await dialog.find(".start-input").setValue("2026-12-01");
    expect((dialog.find(".end-input").element as HTMLInputElement).value).toBe("2027-03-31");

    // 手动改过之后不再自动覆盖
    await dialog.find(".end-input").setValue("2027-05-01");
    await dialog.find(".start-input").setValue("2026-12-10");
    expect((dialog.find(".end-input").element as HTMLInputElement).value).toBe("2027-05-01");
  });

  it("开始冬眠被后端拒绝（如重叠）时弹窗内展示原因且弹窗保持", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .hib-btn').trigger("click");

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_hibernation") {
        throw "与该窝既有冬眠段时间重叠（相邻两段至少要隔一天）";
      }
      if (cmd === "list_colonies") return currentColonies;
      if (cmd === "list_locations") return locations;
      return null;
    });

    await wrapper.find(".hibernation-dialog .submit-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".hibernation-dialog .form-error").text()).toContain("重叠");
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(true);
  });

  it("冬眠卡「确认出眠」：实际结束默认今天可改，提交 confirm_wake（实际≠预计没关系）", async () => {
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "2026-09-25" });
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="3"] .wake-btn').trigger("click");

    const dialog = wrapper.find(".hibernation-dialog");
    expect(dialog.find("h3").text()).toContain("确认出眠");
    expect((dialog.find(".actual-input").element as HTMLInputElement).value).toBe(todayIso());
    await dialog.find(".actual-input").setValue("2026-09-12");

    invokeMock.mockClear();
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("confirm_wake", {
      colonyId: 3,
      actualEndDate: "2026-09-12",
    });
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });

  it("「补录冬眠」：空日期前端拦截；两段日期齐后提交 add_past_hibernation", async () => {
    const wrapper = await mountApp();
    await wrapper.find('.card[data-colony-id="1"] .past-btn').trigger("click");

    const dialog = wrapper.find(".hibernation-dialog");
    expect(dialog.find("h3").text()).toContain("补录");

    invokeMock.mockClear();
    // 空日期直接提交：前端拦截，不发 IPC
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();
    expect(dialog.find(".form-error").text()).toContain("日期");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "add_past_hibernation")).toBe(false);

    await dialog.find(".past-start-input").setValue("2025-12-01");
    await dialog.find(".past-end-input").setValue("2026-02-01");
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("add_past_hibernation", {
      colonyId: 1,
      startDate: "2025-12-01",
      endDate: "2026-02-01",
    });
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(false);
  });

  it("冬眠卡仍可记账：静音块点击照常发 log_care（验收 5：灰化静音但可记账）", async () => {
    const feedOverdue: ColonyAction = {
      action_id: 1,
      name: "喂食",
      icon: null,
      kind: "reminding",
      is_feeding: false,
      suggested_interval_days: 3,
      days_since_last: 4,
      overdue: true,
    };
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "2099-01-01" });
    currentColonies = currentColonies.map((c) => (c.id === 3 ? { ...c, actions: [feedOverdue] } : c));
    const wrapper = await mountApp();

    invokeMock.mockClear();
    await wrapper.find('.card[data-colony-id="3"] .tile[data-action-id="1"]').trigger("click");
    await flushPromises();

    const logCall = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(logCall).toBeDefined();
    expect((logCall![1] as { input: { colony_id: number } }).input.colony_id).toBe(3);
  });
});
