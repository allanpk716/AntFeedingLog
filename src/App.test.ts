import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import App from "./App.vue";
import pkg from "../package.json";
import type { AppSettings, CareActionItem, Colony, ColonyAction, FoodItem, LocationItem, RecentLog } from "./types";
import { addDays, todayIso, todayLabel } from "./lib/dates";
import { stopTodayClock } from "./lib/today";
import DateTimeField from "./components/DateTimeField.vue";
import DatePickerPop from "./components/DatePickerPop.vue";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的
// invokeMock，调用形状 (命令名, 入参) 与旧式 vi.mock("@tauri-apps/api/core") 一致）；
// 事件订阅（db-restored 等）走 mock 工厂内置的立即退订空桩
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("./lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("./testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

// 票 07：统计页图表在 happy-dom 无 canvas，mock 掉 echarts（导航测试只验接线）
const { echartsSetOption } = vi.hoisted(() => ({ echartsSetOption: vi.fn() }));
vi.mock("echarts", () => ({
  init: vi.fn(() => ({ setOption: echartsSetOption, dispose: vi.fn(), resize: vi.fn() })),
}));

/** 票 06：get_settings 的回放数据（测试里可整体替换）；票 11 增凭据两键 */
const defaultSettings: AppSettings = {
  notify_master_enabled: true,
  notify_overdue_enabled: true,
  notify_hibernation_enabled: true,
  wake_remind_days_ahead: 7,
  autostart_enabled: true,
  pushover_user: "",
  pushover_token: "",
};
let currentSettings: AppSettings = defaultSettings;

const locations: LocationItem[] = [
  { id: 1, name: "家", enabled: true, sort: 1 },
  { id: 2, name: "公司", enabled: true, sort: 2 },
];

const foods: FoodItem[] = [
  { id: 1, name: "种子", enabled: true, sort: 1, referenced: false, is_preset: true, category: "seed" },
  { id: 2, name: "干虾仁", enabled: true, sort: 2, referenced: false, is_preset: true, category: "protein" },
  { id: 3, name: "面包虫", enabled: true, sort: 3, referenced: false, is_preset: true, category: "protein" },
  { id: 4, name: "蚕蛹", enabled: false, sort: 4, referenced: false, is_preset: false, category: "protein" },
];

const actions: CareActionItem[] = [
  { id: 1, name: "喂食", icon: null, kind: "reminding", is_feeding: true, suggested_interval_days: 3, enabled: true, sort: 1, referenced: false, is_preset: true },
  { id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 2, referenced: true, is_preset: true },
  { id: 3, name: "巢穴保湿", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 3, referenced: false, is_preset: true },
  { id: 4, name: "垃圾清理", icon: null, kind: "reminding", is_feeding: false, suggested_interval_days: 7, enabled: true, sort: 4, referenced: false, is_preset: true },
  // 自建被引用行：保住规则 10 的 UI 分支（删除禁用 + 「只能停用」标题）不被 F2 预置断言淹没
  { id: 5, name: "降温", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 5, referenced: true, is_preset: false },
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
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
      case "list_food_categories":
        return [
          { category: "seed", interval_days: 7 },
          { category: "protein", interval_days: 3 },
          { category: "sugar", interval_days: 3 },
        ];
      case "list_actions":
        return actions;
      case "list_logs":
        return { total: 0, rows: [] };
      case "photo_wall":
        return []; // 照片墙页挂载即拉载荷；默认空库（页面自渲染空态）
      case "get_settings":
        return currentSettings;
      case "colony_month_records":
        return []; // 打卡/喂食弹窗挂载即拉当月标记；默认空月（黄条/标记用例各自覆写）
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
  currentSettings = defaultSettings;
  // 桌面 WebView 形态为默认（桌面专属入口按 isTauri 渲染）；
  // 浏览器形态在「浏览器模式入口可见性」describe 里单独删掉注入
  (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  baseMock();
});

describe("首页卡片墙", () => {
  it("手机竖屏断点类：卡片墙挂 vp-cards（≤480px 单列的媒体查询落点，webui-checkin 票 08）", async () => {
    const wrapper = await mountApp();
    const cards = wrapper.findAll(".vp-cards");
    expect(cards.length).toBeGreaterThanOrEqual(1);
    expect(cards[0].classes()).toContain("cards");
  });

  it("顶栏品牌旁显示版本号（与 package.json 同源，双端可见）", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find(".brand .ver").text()).toBe(`v${pkg.version}`);
  });

  it("按地点清单顺序分组，卡片含名字/物种徽章/状态徽章/饲养天数大数字", async () => {
    const wrapper = await mountApp();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    expect(invokeMock).toHaveBeenCalledWith("list_locations");

    const titles = wrapper.findAll(".group-title").map((t) => t.text());
    expect(titles).toEqual(["家", "公司"]);

    const home = wrapper.find('.card[data-colony-id="1"]');
    expect(home.find(".bname").text()).toBe("大头一号");
    expect(home.find(".bchip").text()).toBe("大头收获蚁");
    expect(home.find(".bchip.st").text()).toContain("活跃");
    expect(home.find(".bdays .n").text()).toBe("241");
    // avatar-banner 乙-2：天数叠照片下沿「241 天」，无「已饲养 / 天」文案、无开始日期行
    expect(home.find(".bdays").text()).toContain("241");
    expect(home.text()).not.toContain("已饲养 / 天");
    expect(home.text()).not.toContain("开始饲养 2026-01-20");

    const corpCards = wrapper.findAll(".group")[1].findAll(".card");
    expect(corpCards.map((c) => c.find(".bname").text())).toEqual(["针毛一号", "大头二号"]);
  });

  it("冬眠中的窝显示冬眠状态徽章", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find('.card[data-colony-id="3"] .bchip.st').text()).toContain("冬眠");
  });

  it("卡片显示最新巢况数与距上次登记天数（webui-checkin 票 02）；「巢况」按钮打开时间线", async () => {
    currentColonies = colonies.map((c) =>
      c.id === 1
        ? {
            ...c,
            checkin: {
              latest: {
                id: 3,
                colony_id: 1,
                date: "2026-09-15",
                queen_count: 2,
                worker_count: 3000,
                moved_nest: false,
                note: "",
                created_at: "2026-09-15 21:00:00",
                photos: [],
              },
              baseline_date: "2026-09-01",
              days_since_last: 3,
            },
          }
        : c,
    );
    const wrapper = await mountApp();
    const card = wrapper.find('.card[data-colony-id="1"]');
    expect(card.find(".checkin-line").text()).toBe("巢况：蚁后 2 · 工蚁 3000 · 距上次登记 3 天");

    // 打开巢况时间线弹窗：按窝拉时间线
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_colonies":
          return currentColonies;
        case "list_locations":
          return locations;
        case "list_checkins":
          return [];
        default:
          return null;
      }
    });
    await card.find(".bact-btn").trigger("click"); // 横幅右上角「巢况」按钮
    await flushPromises();
    expect(wrapper.find(".checkin-dialog").exists()).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("list_checkins", { colonyId: 1 });
    // 空态占位（checkin-photo-entry 票 03）：拍一张大按钮成为主角，次级链接进完整登记
    expect(wrapper.find(".checkin-dialog .empty-snap-btn").text()).toContain("拍一张巢况照片");
    expect(wrapper.find(".checkin-dialog .empty-full-link").text()).toContain("完整登记");
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
    expect(endedCard.findAll(".bchip").length).toBe(1); // 无物种只留状态徽章
    expect(endedCard.find(".bchip.st").text()).toContain("已结束");
  });

  it("没有任何窝时显示「暂无窝」并保留新建入口", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_colonies" ? [] : cmd === "list_locations" ? locations : null,
    );
    const wrapper = await mountApp();
    expect(wrapper.find(".empty").text()).toContain("暂无窝");
    // 新建窝入口挪到顶栏（交互第三轮 #7）
    expect(wrapper.find(".topbar .new-top-btn").exists()).toBe(true);
  });

  it("紧凑卡片：⋯ 菜单默认隐藏，点开可见编辑/冬眠入口（交互第三轮 #7）", async () => {
    const wrapper = await mountApp();
    const card = wrapper.find('.card[data-colony-id="1"]');
    const menu = card.find(".card-menu");
    expect(menu.exists()).toBe(true);
    expect((menu.element as HTMLElement).style.display).toBe("none");
    await card.find(".dots").trigger("click");
    expect((menu.element as HTMLElement).style.display).not.toBe("none");
    expect(menu.find(".edit-btn").exists()).toBe(true);
    expect(menu.find(".hib-btn").exists()).toBe(true);
  });
});

describe("新建窝", () => {
  it("填表提交调用 create_colony（trim 后的 snake_case 入参），成功后关弹窗并刷新", async () => {
    const wrapper = await mountApp();
    await wrapper.find(".new-top-btn").trigger("click");

    const dialog = wrapper.find(".dialog");
    expect(dialog.exists()).toBe(true);
    expect((dialog.find(".name-input").element as HTMLInputElement).value).toBe("");

    await dialog.find(".name-input").setValue("  新窝一号  ");
    await dialog.find(".species-input").setValue("针毛收获蚁");
    await dialog.find(".location-select").setValue("1");
    await dialog.findComponent(DatePickerPop).vm.$emit("update:modelValue", "2026-09-18");

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
        hydration_method: null,
        interval_changes: [],
      },
    });
    expect(wrapper.find(".dialog").exists()).toBe(false);
    const refreshCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_colonies");
    expect(refreshCalls.length).toBeGreaterThanOrEqual(1);
  });

  it("重名（含首尾空格差异）被前端拦截，不发起 create_colony", async () => {
    const wrapper = await mountApp();
    await wrapper.find(".new-top-btn").trigger("click");

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
    expect(dialog.findComponent(DatePickerPop).props("modelValue")).toBe("2026-01-20");

    await dialog.find(".name-input").setValue("大头一号B");
    await dialog.find(".status-select").setValue("ended");

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
        status: "ended",
        hydration_method: null,
        interval_changes: [],
      },
    });
  });

  it("状态下拉只有 活跃/已结束（冬眠走「开始冬眠」流程，定点修 5）；编辑冬眠窝时该状态禁用显示且保存不改变它", async () => {
    // 新建：无「冬眠中」选项
    const wrapper = await mountApp();
    await wrapper.find(".new-top-btn").trigger("click");
    let options = wrapper.findAll(".status-select option");
    expect(options.map((o) => o.text())).toEqual(["活跃", "已结束"]);

    // 编辑冬眠中的窝：多一个禁用的「冬眠中」项用于显示当前状态
    await wrapper.find(".dialog .cancel-btn").trigger("click");
    await wrapper.find('.card[data-colony-id="3"] .edit-btn').trigger("click");
    const select = wrapper.find(".status-select");
    options = select.findAll("option");
    // 活跃 + 已结束 + 当前冬眠态（禁用显示）
    expect(options.length).toBe(3);
    const hibernatingOption = options.find(
      (o) => (o.element as HTMLOptionElement).value === "hibernating",
    );
    expect(hibernatingOption).toBeDefined();
    expect((hibernatingOption!.element as HTMLOptionElement).disabled).toBe(true);
    expect(hibernatingOption!.text()).toContain("冬眠");
    expect((select.element as HTMLSelectElement).value).toBe("hibernating");

    // 只改名字保存：状态保持冬眠（不经编辑窗解档）
    await wrapper.find(".dialog .name-input").setValue("大头二号B");
    invokeMock.mockClear();
    await wrapper.find(".dialog .submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("update_colony", {
      id: 3,
      input: expect.objectContaining({ status: "hibernating" }),
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

  it("操作 tab：列出全部操作，四个预置行删除一律禁用并提示可停用（反馈第二轮 F2）", async () => {
    const wrapper = await mountApp();
    const dlg = await openSettings(wrapper);

    const rows = dlg.findAll(".dict-row");
    expect(
      rows.map((r) => (r.find(".name-input").element as HTMLInputElement).value),
    ).toEqual(["喂食", "活动区换水", "巢穴保湿", "垃圾清理", "降温"]);

    // 四个预置操作删除全部禁用（含未被引用的巢穴保湿），标题提示可停用（F2）
    for (const row of rows.slice(0, 4)) {
      expect(row.find(".erase-btn").attributes("disabled")).toBeDefined();
      expect(row.find(".erase-btn").attributes("title")).toContain("预置项不能删除");
    }

    // 自建被引用行（规则 10）：删除同样禁用，标题走引用文案而非预置文案
    const custom = rows[4];
    expect(custom.find(".erase-btn").attributes("disabled")).toBeDefined();
    expect(custom.find(".erase-btn").attributes("title")).toContain("只能停用");
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
    expect(dlg.findAll(".dict-row").length).toBe(6);
    // 新增行非预置、未被引用：删除可用（F2 只禁预置与被引用）
    expect(dlg.findAll(".dict-row")[5].find(".erase-btn").attributes("disabled")).toBeUndefined();

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
        sort: 5,
      },
    });
  });

  // ── 食物 tab ──

  it("食物 tab：改名保存走 save_food；大类周期回显并随保存落库（ADR 0008）；停用即时生效；停用行置灰", async () => {
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
    // 预置食物删除禁用、非预置未被引用的「蚕蛹」删除可用（F2）
    expect(rows[0].find(".erase-btn").attributes("disabled")).toBeDefined();
    expect(rows[3].find(".erase-btn").attributes("disabled")).toBeUndefined();

    await rows[0].find(".name-input").setValue("瓜子");
    // ADR 0008：食物行只剩撤食间隔输入框（种子不易腐，置灰）；大类周期区回显三行（种子 7 / 蛋白质 3 / 糖水 3）
    const rowInputs = rows[0].findAll("input[type=number]");
    expect(rowInputs.length).toBe(1);
    expect((rowInputs[0].element as HTMLInputElement).disabled).toBe(true);
    const catInputs = dlg.findAll(".cat-interval-row .interval-input");
    expect(catInputs.map((i) => (i.element as HTMLInputElement).value)).toEqual(["7", "3", "3"]);
    expect(dlg.text()).toContain("提醒按大类计");
    await catInputs[1].setValue("5");
    invokeMock.mockClear();
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("save_food", {
      input: { id: 1, name: "瓜子", sort: 0, perishable: false, retrieval_hours: null, category: "seed" },
    });
    // 大类周期三条逐一落库（改过的蛋白质=5，其余原值照传）
    expect(invokeMock).toHaveBeenCalledWith("save_food_category_interval", {
      category: "protein",
      intervalDays: 5,
    });
    expect(invokeMock).toHaveBeenCalledWith("save_food_category_interval", {
      category: "seed",
      intervalDays: 7,
    });
    expect(invokeMock).toHaveBeenCalledWith("save_food_category_interval", {
      category: "sugar",
      intervalDays: 3,
    });

    invokeMock.mockClear();
    await dlg.findAll(".dict-row")[1].find(".row-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_food_enabled", { id: 2, enabled: false });
  });
});

describe("网页端首启向导（webui-checkin 票 03）", () => {
  const wizardConfig = {
    enabled: false,
    segments: [],
    port: 17321,
    token: "0123456789abcdef0123456789abcdef",
    token_generated_at: "2026-09-19 08:00:00",
  };

  it("get_webui_wizard_done 返回 false（未做）→ 启动即弹一次向导；完成/跳过写键后关闭", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_colonies":
          return [];
        case "list_locations":
          return locations;
        case "get_webui_wizard_done":
          return false;
        case "get_webui_config":
          return wizardConfig;
        case "list_network_segments":
          return [{ cidr: "100.84.0.0/16", encrypted_mesh: true, label: "NetBird 虚拟网" }];
        default:
          return null;
      }
    });
    const wrapper = await mountApp();

    const wizard = wrapper.find(".webui-wizard");
    expect(wizard.exists()).toBe(true);
    expect(wizard.text()).toContain("首次设置向导");

    // 跳过：写完成键 + 关闭（不发任何保存）
    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "mark_webui_wizard_done" ? null : null,
    );
    await wrapper.find(".wiz-skip-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("mark_webui_wizard_done");
    expect(wrapper.find(".webui-wizard").exists()).toBe(false);
  });

  it("get_webui_wizard_done 返回 true（已做）→ 不弹向导", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_colonies":
          return [];
        case "list_locations":
          return locations;
        case "get_webui_wizard_done":
          return true;
        default:
          return null;
      }
    });
    const wrapper = await mountApp();
    expect(wrapper.find(".webui-wizard").exists()).toBe(false);
  });
});

describe("顶栏导航（票 07/08）", () => {
  it("顶栏显示今天日期（YYYY-MM-DD 周X，票 09 停靠 F 对齐 mock）", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find(".topbar .today").exists()).toBe(true);
    expect(wrapper.find(".topbar .today").text()).toBe(todayLabel());
  });

  it("四页 nav：首页/统计/照片/记录可切换，照片页拉照片墙、记录页拉记录列表", async () => {
    const wrapper = await mountApp();

    const tabs = wrapper.findAll(".topbar .tab");
    expect(tabs.map((t) => t.text())).toEqual(["首页", "统计", "照片", "记录"]);
    expect(tabs[0].classes()).toContain("active");

    // 切到统计：统计页渲染并拉数据
    invokeMock.mockClear();
    await tabs[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".stats-page").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "get_stats")).toBe(true);

    // 切到照片（窝头像票 05）：照片墙页渲染并拉载荷
    invokeMock.mockClear();
    await wrapper.findAll(".topbar .tab")[2].trigger("click");
    await flushPromises();
    expect(wrapper.find(".photo-wall").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "photo_wall")).toBe(true);

    // 切到记录（票 08）：记录列表页渲染并拉数据
    invokeMock.mockClear();
    await wrapper.findAll(".topbar .tab")[3].trigger("click");
    await flushPromises();
    expect(wrapper.find(".log-list").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(true);

    // 切回首页：卡片墙回来
    await wrapper.findAll(".topbar .tab")[0].trigger("click");
    expect(wrapper.find(".group-title").exists()).toBe(true);
    expect(wrapper.find(".stats-page").exists()).toBe(false);
    expect(wrapper.find(".photo-wall").exists()).toBe(false);
    expect(wrapper.find(".log-list").exists()).toBe(false);
  });

  it("外壳：顶栏之外有独立滚动容器 .page-body（交互第三轮 #6）", async () => {
    const wrapper = await mountApp();
    expect(wrapper.find(".page-body").exists()).toBe(true);
    expect(wrapper.find(".topbar").exists()).toBe(true);
  });

  it("记录页里改动记录后抛 changed：首页数据即时重算（票 08 验收 5 的接线）", async () => {
    const wrapper = await mountApp();
    await wrapper.findAll(".topbar .tab")[3].trigger("click");
    await flushPromises();

    invokeMock.mockClear();
    wrapper.findComponent({ name: "LogListPage" }).vm.$emit("changed");
    await flushPromises();

    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
  });
});

// ── 主界面三页保活（票 04）：再进即时——不重挂载、不重发页面级 IPC ──

describe("主界面三页保活（票 04）", () => {
  it("统计页切走再进：不重挂载，get_stats 不重发", async () => {
    const wrapper = await mountApp();
    const tabs = () => wrapper.findAll(".topbar .tab");

    await tabs()[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".stats-page").exists()).toBe(true);
    const firstVisits = invokeMock.mock.calls.filter(([cmd]) => cmd === "get_stats").length;
    expect(firstVisits).toBeGreaterThanOrEqual(1);

    invokeMock.mockClear();
    await tabs()[0].trigger("click"); // 切回首页：统计页失活（离屏缓存）
    await flushPromises();
    expect(wrapper.find(".stats-page").exists()).toBe(false);
    await tabs()[1].trigger("click"); // 再进统计
    await flushPromises();

    expect(wrapper.find(".stats-page").exists()).toBe(true);
    // 保活：实例与数据保留，不重新挂载、不重发页面级 IPC
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "get_stats")).toBe(false);
  });

  it("照片页切走再进：不重挂载，photo_wall 不重发", async () => {
    const wrapper = await mountApp();
    const tabs = () => wrapper.findAll(".topbar .tab");

    await tabs()[2].trigger("click");
    await flushPromises();
    expect(wrapper.find(".photo-wall").exists()).toBe(true);

    invokeMock.mockClear();
    await tabs()[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".photo-wall").exists()).toBe(false);
    await tabs()[2].trigger("click");
    await flushPromises();

    expect(wrapper.find(".photo-wall").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "photo_wall")).toBe(false);
  });

  it("记录页切走再进：不重挂载，list_logs 不重发，筛选状态在往返间保留", async () => {
    const wrapper = await mountApp();
    const tabs = () => wrapper.findAll(".topbar .tab");

    await tabs()[3].trigger("click");
    await flushPromises();
    expect(wrapper.find(".log-list").exists()).toBe(true);
    // 选地点筛选（即改即查，会再发一次 list_logs），随后往返
    await wrapper.find(".f-location").setValue("1");
    await flushPromises();

    invokeMock.mockClear();
    await tabs()[0].trigger("click");
    await flushPromises();
    await tabs()[3].trigger("click");
    await flushPromises();

    expect(wrapper.find(".log-list").exists()).toBe(true);
    // 保活：不重发页面级 IPC；筛选状态保留（预期行为，规格 Further Notes）
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_logs")).toBe(false);
    expect((wrapper.find(".f-location").element as HTMLSelectElement).value).toBe("1");
  });

  it("首页不进 KeepAlive：数据在根组件，往返后照常渲染", async () => {
    const wrapper = await mountApp();
    const tabs = () => wrapper.findAll(".topbar .tab");

    await tabs()[2].trigger("click");
    await flushPromises();
    await tabs()[0].trigger("click");
    await flushPromises();

    expect(wrapper.find(".group-title").exists()).toBe(true);
    expect(wrapper.find(".photo-wall").exists()).toBe(false);
    expect(tabs()[0].classes()).toContain("active");
  });
});

describe("设置 · 通知（票 06）", () => {
  async function openNotifyTab(wrapper: Awaited<ReturnType<typeof mountApp>>) {
    await wrapper.find(".settings-btn").trigger("click");
    await flushPromises();
    const dlg = wrapper.find(".settings-dialog");
    await dlg.find(".tab-notify").trigger("click");
    await flushPromises();
    return dlg;
  }

  it("通知 tab：回显总开关与提前天数，保存发出 set_settings 并刷新首页", async () => {
    const wrapper = await mountApp();
    const dlg = await openNotifyTab(wrapper);

    const boxes = dlg.findAll(".notify-row input[type=checkbox]");
    // 总开关 + 撤食提醒（票 05）+ 开机自启（票 09）；分类子开关已作废（反馈第二轮 Q7/Q9）
    expect(boxes.length).toBe(3);
    expect((boxes[0].element as HTMLInputElement).checked).toBe(true);
    // 撤食提醒默认开
    expect((boxes[1].element as HTMLInputElement).checked).toBe(true);
    // 开机自启默认开
    expect((boxes[2].element as HTMLInputElement).checked).toBe(true);
    expect((dlg.find(".days-input").element as HTMLInputElement).value).toBe("7");

    await dlg.find(".days-input").setValue("3");

    invokeMock.mockClear();
    // set_settings 回显保存后的设置（Rust 返回收敛后的生效值）
    invokeMock.mockResolvedValueOnce(currentSettings);
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_settings", {
      input: {
        notify_master_enabled: true,
        notify_overdue_enabled: true, // 分类开关作废：前端固定回写 true（键保留在库里）
        notify_hibernation_enabled: true,
        notify_retrieval_enabled: true, // 撤食提醒开关（票 05）
        wake_remind_days_ahead: 3,
        autostart_enabled: true,
        pushover_user: "", // 票 11：凭据随表单整体回写（未填 = 空串，回落环境变量）
        pushover_token: "",
      },
    });
    // changed → 外层刷新首页
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
    // 刷新重渲染后重查弹窗（旧 wrapper 可能已脱离文档）
    const dlgAfter = wrapper.find(".settings-dialog");
    expect(dlgAfter.find(".saved-hint").text()).toContain("已保存");
  });

  it("通知 tab：开机自启开关随保存一起落库（票 09）", async () => {
    const wrapper = await mountApp();
    const dlg = await openNotifyTab(wrapper);

    const boxes = dlg.findAll(".notify-row input[type=checkbox]");
    await boxes[2].setValue(false); // 自启在撤食提醒开关之后（票 05 加了一档）

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce({ ...currentSettings, autostart_enabled: false });
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_settings", {
      input: expect.objectContaining({ autostart_enabled: false }),
    });
    // 回显保存后的生效值
    const boxesAfter = wrapper
      .find(".settings-dialog")
      .findAll(".notify-row input[type=checkbox]");
    expect((boxesAfter[2].element as HTMLInputElement).checked).toBe(false);
  });

  it("通知 tab：提前天数非法时报错且不落库", async () => {
    const wrapper = await mountApp();
    const dlg = await openNotifyTab(wrapper);

    await dlg.find(".days-input").setValue("-1");
    await dlg.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(dlg.find(".form-error").text()).toContain("0–365");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "set_settings")).toBe(false);
  });

  it("通知 tab：发送测试通知按钮双通道回显结果（反馈第二轮 F4）", async () => {
    const wrapper = await mountApp();
    const dlg = await openNotifyTab(wrapper);

    invokeMock.mockClear();
    // 未配置 Pushover：pushover=null（桌面真发在 Rust 侧，这里只验前端接线）
    invokeMock.mockResolvedValueOnce({ desktop_ok: true, desktop_error: null, pushover: null });
    await dlg.find(".tab-body .btn:not(.primary)").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("send_test_notification");
    expect(dlg.find(".saved-hint").text()).toContain("测试结果：桌面 ✓ · 手机：未配置");
  });
});

describe("设置 · 数据与备份（票 09）", () => {
  async function openDataTab(wrapper: Awaited<ReturnType<typeof mountApp>>) {
    await wrapper.find(".settings-btn").trigger("click");
    await flushPromises();
    const dlg = wrapper.find(".settings-dialog");
    await dlg.find(".tab-data").trigger("click");
    await flushPromises();
    return dlg;
  }

  it("数据 tab：帮助文案写明手动拷贝备份需先从托盘真实退出、应用内备份无需退出", async () => {
    const wrapper = await mountApp();
    const dlg = await openDataTab(wrapper);

    const body = dlg.find(".tab-body");
    expect(body.text()).toContain("手动拷贝备份需先从托盘真实退出");
    expect(body.text()).toContain("应用内一键安全备份无需退出");
    expect(dlg.find(".reveal-btn").exists()).toBe(true);
    expect(dlg.find(".backup-btn").exists()).toBe(true);
    expect(dlg.find(".export-csv-btn").exists()).toBe(true);
    expect(dlg.find(".export-json-btn").exists()).toBe(true);
  });

  it("「打开数据文件夹」发 reveal_data_folder；「安全备份」发 backup_to 并展示备份路径", async () => {
    const wrapper = await mountApp();
    const dlg = await openDataTab(wrapper);

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce("C:\\Users\\x\\AppData\\Roaming\\com.antfeedinglog.app");
    await dlg.find(".reveal-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("reveal_data_folder");

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce("D:\\backup\\ant-feeding-log-backup-20260918-091530.zip");
    await dlg.find(".backup-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("backup_to");
    expect(dlg.find(".data-result").text()).toContain("ant-feeding-log-backup-20260918-091530.zip");

    // 用户在对话框取消（Rust 返回 null）：不报错也不留旧结果
    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce(null);
    await dlg.find(".backup-btn").trigger("click");
    await flushPromises();
    expect(dlg.find(".data-error").exists()).toBe(false);
    expect(dlg.find(".data-result").exists()).toBe(false);
  });

  it("「导出 CSV」「导出 JSON」发 export_data 带格式并展示产物路径", async () => {
    const wrapper = await mountApp();
    const dlg = await openDataTab(wrapper);

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce("D:\\arch\\ant-feeding-log-export-20260918.csv");
    await dlg.find(".export-csv-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("export_data", { format: "csv" });

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce("D:\\arch\\ant-feeding-log-export-20260918.json");
    await dlg.find(".export-json-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("export_data", { format: "json" });
    expect(dlg.find(".data-result").text()).toContain("export-20260918.json");
  });

  it("备份/导出失败时展示原因，不误报成功", async () => {
    const wrapper = await mountApp();
    const dlg = await openDataTab(wrapper);

    invokeMock.mockClear();
    invokeMock.mockRejectedValueOnce("备份拷贝失败: 磁盘已满");
    await dlg.find(".backup-btn").trigger("click");
    await flushPromises();

    expect(dlg.find(".data-error").text()).toContain("磁盘已满");
    expect(dlg.find(".data-result").exists()).toBe(false);
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
    foods: [],
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
    foods: [],
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
    foods: [],
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
    foods: [],
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
    foods: [],
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

  it("喂食块带大类与食物明细：大类层超期整块红，悬停 title 大类口径在前、逐食物参考在后（ADR 0008）", async () => {
    const feedWithFoods: ColonyAction = {
      ...feedOverdue,
      days_since_last: 8,
      foods: [
        { food_id: 1, name: "种子", days_since_last: 8 },
        { food_id: 2, name: "虾干", days_since_last: 1 },
        { food_id: 3, name: "面包虫干", days_since_last: null },
      ],
      categories: [
        { category: "seed", interval_days: 7, days_since_last: 8, overdue: true },
        { category: "protein", interval_days: 3, days_since_last: 1, overdue: false },
        { category: "sugar", interval_days: 3, days_since_last: null, overdue: false },
      ],
    };
    colony1With([feedWithFoods, waterReg]);
    const wrapper = await mountApp();

    const feed = wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="1"]');
    expect(feed.classes()).toContain("bad");
    expect(feed.find(".pill").text()).toBe("⚠ 超期 5 天");
    expect(feed.attributes("title")).toBe(
      "种子：距上次 8 天 / 周期 7 天 · 超期\n" +
        "蛋白质：距上次 1 天 / 周期 3 天\n" +
        "糖水：尚未记录 / 周期 3 天\n" +
        "种子：距上次 8 天\n" +
        "虾干：距上次 1 天\n" +
        "面包虫干：尚未记录\n" +
        "（提醒按大类计，逐食物仅供参考）",
    );

    // 非喂食块无食物明细 → 不带 title
    const water = wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]');
    expect(water.attributes("title")).toBeUndefined();
  });

  it("喂食块大类层顶红（统一层新鲜）：pill 报「该喂大类了」（ADR 0008）", async () => {
    const feedWithFoods: ColonyAction = {
      ...feedOverdue,
      days_since_last: 2,
      overdue: true,
      effective_interval_days: 3,
      foods: [
        { food_id: 1, name: "种子", days_since_last: 2 },
        { food_id: 3, name: "面包虫干", days_since_last: 8 },
      ],
      categories: [
        { category: "seed", interval_days: 7, days_since_last: 2, overdue: false },
        { category: "protein", interval_days: 3, days_since_last: 8, overdue: true },
      ],
    };
    colony1With([feedWithFoods, waterReg]);
    const wrapper = await mountApp();

    const feed = wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="1"]');
    expect(feed.classes()).toContain("bad");
    expect(feed.find(".pill").text()).toBe("⚠ 该喂蛋白质了");
  });

  it("非喂食弹打卡面板：默认今天可补录，点「记录」才落库并刷新", async () => {
    colony1With([waterReg]);
    const wrapper = await mountApp();

    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
    const dialog = wrapper.find(".quick-dialog");
    expect(dialog.exists()).toBe(true);
    expect(dialog.find("h3").text()).toBe("记录活动区换水 · 大头一号");

    await dialog.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-17T21:00");
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

  it("点喂食类操作弹食物多选弹窗（只认 is_feeding 位）：只列启用食物，勾两种 + 补录时间 + 备注，确认生成一条带两食物的记录", async () => {
    // 操作名「投喂」不含「喂食」：弹窗触发与标题跟随 is_feeding/名字，不受字典改名影响
    colony1With([feedCustom]);
    const wrapper = await mountApp();

    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="9"]').trigger("click");

    const dialog = wrapper.find(".feed-dialog");
    expect(dialog.exists()).toBe(true);
    expect(dialog.find("h3").text()).toBe("记录投喂 · 大头一号");
    expect(invokeMock).toHaveBeenCalledWith("list_foods");
    // 票 07：字典首读完成前食物区是「加载中…」占位——flush 后 chips 才渲染
    await flushPromises();

    // 停用食物不出现在新建记录入口（规则 10）
    const chips = dialog.findAll(".food");
    expect(chips.map((c) => c.text())).toEqual(["种子", "干虾仁", "面包虫"]);

    await chips[1].trigger("click"); // 干虾仁
    await chips[2].trigger("click"); // 面包虫
    expect(chips[1].classes()).toContain("selected");
    expect(chips[0].classes()).not.toContain("selected");

    // 补录昨天时间（验收 3：距上次按发生时间算，Rust 侧覆盖计算）
    await dialog.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-17T21:00");
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
      if (cmd === "list_actions") return actions; // 弹窗 markers 名字表全量来源（终局评审口径）
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

  it("打卡面板提交失败：原因展示在面板内、面板不关", async () => {
    colony1With([waterReg]);
    const wrapper = await mountApp();
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "log_care": throw "操作「活动区换水」已停用，不能新记";
        case "list_colonies": return currentColonies;
        case "list_locations": return locations;
        case "colony_month_records": return []; // 弹窗挂载拉当月标记，给空月
        case "list_actions": return actions; // 弹窗 markers 名字表全量来源（终局评审口径）
        default: return null;
      }
    });
    await wrapper.find('.card[data-colony-id="1"] .tile[data-action-id="2"]').trigger("click");
    await wrapper.find(".quick-dialog .record-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".quick-dialog .form-error").text()).toContain("停用");
    expect(wrapper.find(".quick-dialog").exists()).toBe(true);
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
    const start = dialog.findAllComponents(DatePickerPop)[0].props("modelValue") as string;
    expect(start).toBe(todayIso());
    expect(dialog.findAllComponents(DatePickerPop)[1].props("modelValue")).toBe(addDays(start, 120));

    await dialog.findAllComponents(DatePickerPop)[0].vm.$emit("update:modelValue", "2026-12-01");
    // 手动改预计结束（选完开始日自动预填、但用户可改）
    await dialog.findAllComponents(DatePickerPop)[1].vm.$emit("update:modelValue", "2027-04-15");

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
    await dialog.findAllComponents(DatePickerPop)[0].vm.$emit("update:modelValue", "2026-12-01");
    expect(dialog.findAllComponents(DatePickerPop)[1].props("modelValue")).toBe("2027-03-31");

    // 手动改过之后不再自动覆盖
    await dialog.findAllComponents(DatePickerPop)[1].vm.$emit("update:modelValue", "2027-05-01");
    await dialog.findAllComponents(DatePickerPop)[0].vm.$emit("update:modelValue", "2026-12-10");
    expect(dialog.findAllComponents(DatePickerPop)[1].props("modelValue")).toBe("2027-05-01");
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
    expect(dialog.findComponent(DatePickerPop).props("modelValue")).toBe(todayIso());
    await dialog.findComponent(DatePickerPop).vm.$emit("update:modelValue", "2026-09-12");

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

    await dialog.findAllComponents(DatePickerPop)[0].vm.$emit("update:modelValue", "2025-12-01");
    await dialog.findAllComponents(DatePickerPop)[1].vm.$emit("update:modelValue", "2026-02-01");
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("add_past_hibernation", {
      colonyId: 1,
      startDate: "2025-12-01",
      endDate: "2026-02-01",
    });
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(false);
  });

  it("冬眠卡横幅「改期」：预填当前预计出眠日，提交 update_expected_end 并刷新（票 09 停靠 D）", async () => {
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "2026-12-01" });
    const wrapper = await mountApp();

    const card = wrapper.find('.card[data-colony-id="3"]');
    // 改期入口只在冬眠卡横幅上；活跃卡没有
    expect(card.find(".banner .resched-btn").exists()).toBe(true);
    expect(wrapper.find('.card[data-colony-id="1"] .resched-btn').exists()).toBe(false);

    await card.find(".banner .resched-btn").trigger("click");

    const dialog = wrapper.find(".hibernation-dialog");
    expect(dialog.exists()).toBe(true);
    expect(dialog.find("h3").text()).toContain("修改预计出眠");
    expect(dialog.findComponent(DatePickerPop).props("modelValue")).toBe("2026-12-01");

    await dialog.findComponent(DatePickerPop).vm.$emit("update:modelValue", "2027-01-15");
    invokeMock.mockClear();
    await dialog.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("update_expected_end", {
      colonyId: 3,
      newExpectedEndDate: "2027-01-15",
    });
    expect(wrapper.find(".hibernation-dialog").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_colonies")).toBe(true);
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
      foods: [],
    };
    colony3With({ id: 9, start_date: "2026-08-20", expected_end_date: "2099-01-01" });
    currentColonies = currentColonies.map((c) => (c.id === 3 ? { ...c, actions: [feedOverdue] } : c));
    const wrapper = await mountApp();

    invokeMock.mockClear();
    // F1 起：点击先弹打卡面板，点「记录」才落库（灰化静音仍可记账的验收不变）
    await wrapper.find('.card[data-colony-id="3"] .tile[data-action-id="1"]').trigger("click");
    await wrapper.find(".quick-dialog .record-btn").trigger("click");
    await flushPromises();

    const logCall = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(logCall).toBeDefined();
    expect((logCall![1] as { input: { colony_id: number } }).input.colony_id).toBe(3);
  });
});

describe("浏览器模式入口可见性（终局评审 Important → 网页端窝编辑票 02）", () => {
  it("浏览器模式：顶栏无「统计」/设置；新建窝与卡片「编辑」双端可见；首页/照片/记录与巢况入口照常", async () => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const wrapper = await mountApp();

    // 顶栏三 tab：首页/照片/记录（统计是桌面专属页；照片墙双端都有，窝头像票 05）
    const tabs = wrapper.findAll(".topbar .tab");
    expect(tabs.map((t) => t.text())).toEqual(["首页", "照片", "记录"]);
    expect(wrapper.find(".settings-btn").exists()).toBe(false);
    // 网页端窝编辑票 02：「＋ 新建窝」放开双端
    expect(wrapper.find(".new-top-btn").exists()).toBe(true);
    // 卡片「✏️ 编辑窝信息」（ColonyCard）同步放开双端；「巢况」是网页端功能不隐藏
    expect(wrapper.find(".card .edit-btn").exists()).toBe(true);
    expect(wrapper.find(".card .bact-btn").exists()).toBe(true);

    // 网页端「照片」页照常进入并拉载荷
    invokeMock.mockClear();
    await tabs[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".photo-wall").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "photo_wall")).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "get_photo_abs_dir")).toBe(false);
  });

  it("桌面模式：统计 tab / 照片 tab / 设置 / 新建窝 / 卡片编辑照常渲染", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const wrapper = await mountApp();

    expect(wrapper.findAll(".topbar .tab").map((t) => t.text())).toEqual([
      "首页",
      "统计",
      "照片",
      "记录",
    ]);
    expect(wrapper.find(".settings-btn").exists()).toBe(true);
    expect(wrapper.find(".new-top-btn").exists()).toBe(true);
    expect(wrapper.find(".card .edit-btn").exists()).toBe(true);
  });
});

// ── 主页面「检查更新」按钮直达更新页签（update-entry 票 01）──

describe("主页面「检查更新」按钮（update-entry 票 01）", () => {
  it("桌面形态：⚙设置旁有「检查更新」按钮；点击打开设置弹窗直达「更新」页签并自动开始检查", async () => {
    const wrapper = await mountApp();
    const btn = wrapper.find(".topbar .update-top-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.attributes("title")).toContain("检查更新");

    // 设置链照常走 baseMock，只补检查结果
    const base = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "check_update_now" ? { status: "up_to_date" } : base(cmd),
    );
    invokeMock.mockClear();
    await btn.trigger("click");
    await flushPromises();

    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(wrapper.find(".tab-update").classes()).toContain("active");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "check_update_now")).toBe(true);
    expect(wrapper.find(".update-ok").text()).toContain("已是最新");
  });

  it("「⚙设置」打开仍是默认「操作」页签（不因新入口改变旧路径）", async () => {
    const wrapper = await mountApp();
    await wrapper.find(".settings-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".settings-dialog").exists()).toBe(true);
    expect(wrapper.find(".tab-actions").classes()).toContain("active");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "check_update_now")).toBe(false);
  });

  it("浏览器形态：不渲染「检查更新」按钮（更新是桌面端专属）", async () => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const wrapper = await mountApp();
    expect(wrapper.find(".topbar .update-top-btn").exists()).toBe(false);
  });
});

// ── 轻提示宿主挂应用根（保湿方式+轻提示票 03）──

describe("轻提示宿主挂应用根（票 03）", () => {
  it("桌面形态：App 根渲染 .toast-host（全局唯一轻提示出口）", async () => {
    const wrapper = await mountApp();

    expect(wrapper.find(".toast-host").exists()).toBe(true);
  });

  it("网页端与桌面端同一前端：浏览器形态同样渲染 .toast-host（断言挂载即可）", async () => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const wrapper = await mountApp();

    expect(wrapper.find(".toast-host").exists()).toBe(true);
  });
});

// ── 跨天自动刷新（今天时钟源票）──

describe("跨天自动刷新（今天时钟源）", () => {
  it("停在首页跨零点：顶栏日期零操作变新的一天，首页数据重拉（距上次/红标/饲养天数跟上）", async () => {
    // 单例时钟是应用级生命周期：前面用例挂载 App 启动过真表且未卸载，
    // 先停掉，避免本用例的假表启动被幂等守卫挡掉、漏掉启动对表
    stopTodayClock();
    // 只 fake 分钟计时器与系统时钟（仓库先例：不劫持 setImmediate，flushPromises 不挂）
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(new Date(2026, 8, 21, 23, 59, 30));
    const wrapper = await mountApp();
    expect(wrapper.find(".topbar .today").text()).toBe("2026-09-21 周一");

    const before = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_colonies").length;

    // 跨过 00:00，下一次分钟 tick 落在零点后：不点任何按钮，界面自己跟上
    vi.setSystemTime(new Date(2026, 8, 22, 0, 0, 20));
    vi.advanceTimersByTime(60_000);
    await flushPromises();

    expect(wrapper.find(".topbar .today").text()).toBe("2026-09-22 周二");
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "list_colonies").length,
    ).toBeGreaterThan(before);

    stopTodayClock();
    vi.useRealTimers();
  });
});
