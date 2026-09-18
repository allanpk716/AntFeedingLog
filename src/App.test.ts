import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import App from "./App.vue";
import type { Colony, LocationItem } from "./types";

// 不依赖 Tauri 运行时：mock 掉 IPC，按命令名回放数据
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const locations: LocationItem[] = [
  { id: 1, name: "家", enabled: true, sort: 1 },
  { id: 2, name: "公司", enabled: true, sort: 2 },
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
  },
  {
    id: 2,
    name: "针毛一号",
    species: "针毛收获蚁",
    location_id: 2,
    start_date: "2026-06-14",
    status: "active",
    days_raised: 96,
  },
  {
    id: 3,
    name: "大头二号",
    species: "大头收获蚁",
    location_id: 2,
    start_date: "2026-02-10",
    status: "hibernating",
    days_raised: 220,
  },
  {
    id: 4,
    name: "老窝",
    species: null,
    location_id: null,
    start_date: "2025-01-01",
    status: "ended",
    days_raised: 626,
  },
];

function baseMock() {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_colonies":
        return colonies;
      case "list_locations":
        return locations;
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

describe("地点管理", () => {
  async function openManager(wrapper: Awaited<ReturnType<typeof mountApp>>) {
    await wrapper.find(".location-mgr-btn").trigger("click");
    return wrapper.find(".loc-dialog");
  }

  it("列出全部地点，支持上下移调顺序，保存时按新顺序逐行 save_location", async () => {
    const wrapper = await mountApp();
    const dlg = await openManager(wrapper);

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
    expect(wrapper.find(".loc-dialog").exists()).toBe(false);
  });

  it("可新增地点（新行 id 为空），保存时一并提交", async () => {
    const wrapper = await mountApp();
    const dlg = await openManager(wrapper);

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

  it("行级操作不关弹窗：删除被引用展示原因；停用成功后弹窗保持、未保存改名不丢、外层静默刷新", async () => {
    const wrapper = await mountApp();
    const dlg = await openManager(wrapper);

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
        default:
          return null;
      }
    });

    const rows = dlg.findAll(".loc-row");
    await rows[0].find(".loc-erase-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("erase_location", { id: 1 });
    expect(dlg.find(".loc-error").text()).toContain("停用");
    expect(wrapper.find(".loc-dialog").exists()).toBe(true);
    expect(dlg.findAll(".loc-row").length).toBe(2);

    await rows[0].find(".loc-deactivate-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("deactivate_location", { id: 1 });
    // 弹窗仍开着；行内未保存的改名保留；停用态行内可见
    expect(wrapper.find(".loc-dialog").exists()).toBe(true);
    expect(
      (wrapper.find(".loc-row .loc-name-input").element as HTMLInputElement).value,
    ).toBe("老家");
    expect(wrapper.find(".loc-row .loc-disabled-chip").exists()).toBe(true);
    // 行级操作触发外层静默刷新（changed → list_locations），但弹窗不关
    const refreshCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "list_locations");
    expect(refreshCalls.length).toBeGreaterThanOrEqual(1);
  });
});
