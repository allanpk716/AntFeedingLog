import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import LocationManagerPanel from "./LocationManagerPanel.vue";
import type { LocationItem } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的 invokeMock）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

// 轻提示接线（保湿方式+轻提示票 04）：toast 模块整体 mock 掉，断言调用点
// 弹没弹、弹的什么；store/宿主自身行为在 toast.test.ts / ToastHost.test.ts
const { showErrorMock, showSuccessMock } = vi.hoisted(() => ({
  showErrorMock: vi.fn(),
  showSuccessMock: vi.fn(),
}));
vi.mock("../lib/toast", () => ({
  showSuccess: showSuccessMock,
  showError: showErrorMock,
}));

function locFixture(overrides: Partial<LocationItem> = {}): LocationItem {
  return { id: 1, name: "阳台", enabled: true, sort: 0, ...overrides };
}

interface Extra {
  locations?: LocationItem[];
  saveError?: string;
  enableError?: string;
  eraseError?: string;
}

async function openPanel(extra: Extra = {}) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "save_location":
        if (extra.saveError) throw extra.saveError;
        return null;
      case "set_location_enabled":
        if (extra.enableError) throw extra.enableError;
        return null;
      case "erase_location":
        if (extra.eraseError) throw extra.eraseError;
        return null;
      default:
        return null;
    }
  });
  const wrapper = mount(LocationManagerPanel, {
    props: {
      locations:
        extra.locations ?? [locFixture(), locFixture({ id: 2, name: "架上", sort: 1 })],
    },
  });
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  showSuccessMock.mockClear();
  showErrorMock.mockClear();
});

describe("地点管理面板（票 04 轻提示接线）", () => {
  it("渲染：按 sort 排序列出地点行，停用行灰态带徽章", async () => {
    const wrapper = await openPanel({
      locations: [
        locFixture({ id: 2, name: "架上", sort: 1, enabled: false }),
        locFixture({ id: 1, name: "阳台", sort: 0 }),
      ],
    });

    const rows = wrapper.findAll(".loc-row");
    expect(rows.length).toBe(2);
    expect((rows[0]!.find(".loc-name-input").element as HTMLInputElement).value).toBe("阳台");
    expect((rows[1]!.find(".loc-name-input").element as HTMLInputElement).value).toBe("架上");
    expect(rows[1]!.classes()).toContain("row-disabled");
    expect(rows[1]!.find(".loc-disabled-chip").exists()).toBe(true);
    // 停用行的行级按钮切到「启用」
    expect(rows[0]!.find(".loc-deactivate-btn").exists()).toBe(true);
    expect(rows[1]!.find(".loc-activate-btn").exists()).toBe(true);
  });

  it("保存：按行序逐行 save_location，成功弹「已保存」并抛 saved（原成功静默，窗不关视图不变）", async () => {
    const wrapper = await openPanel();
    invokeMock.mockClear();

    await wrapper.find(".save-locations").trigger("click");
    await flushPromises();

    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "save_location")).toEqual([
      ["save_location", { input: { id: 1, name: "阳台", sort: 0 } }],
      ["save_location", { input: { id: 2, name: "架上", sort: 1 } }],
    ]);
    expect(showSuccessMock).toHaveBeenCalledWith("已保存");
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(wrapper.emitted("saved")).toBeTruthy();
  });

  it("保存失败：失败轻提示带原因，内联红字保留为行内上下文通道", async () => {
    const wrapper = await openPanel({ saveError: "库被锁住" });

    await wrapper.find(".save-locations").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("保存失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.find(".loc-error").text()).toContain("库被锁住");
  });

  it("保存校验（空名）维持内联红字，不进轻提示（字段级校验例外）", async () => {
    const wrapper = await openPanel();
    await wrapper.findAll(".loc-name-input")[0]!.setValue("");

    await wrapper.find(".save-locations").trigger("click");
    await flushPromises();

    expect(wrapper.find(".loc-error").text()).toContain("不能为空");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_location")).toBe(false);
  });

  it("保存校验（重名）维持内联红字，不进轻提示", async () => {
    const wrapper = await openPanel();
    await wrapper.findAll(".loc-name-input")[1]!.setValue("阳台");

    await wrapper.find(".save-locations").trigger("click");
    await flushPromises();

    expect(wrapper.find(".loc-error").text()).toContain("已存在");
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_location")).toBe(false);
  });

  it("新增：空名/重名字段级校验维持内联不进轻提示；合法新增为纯本地行不落库不弹", async () => {
    const wrapper = await openPanel();

    // 空名
    await wrapper.find(".loc-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".loc-error").text()).toContain("不能为空");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();

    // 重名
    await wrapper.find(".loc-add-input").setValue("阳台");
    await wrapper.find(".loc-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".loc-error").text()).toContain("已存在");
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();

    // 合法新增：纯本地行，不调 IPC 也不弹（落库在「保存」）
    await wrapper.find(".loc-add-input").setValue("窗台");
    await wrapper.find(".loc-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".loc-row").length).toBe(3);
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("行级停用/启用成功各弹「已停用/已启用」带名字（与票 03 字典行级操作同型）", async () => {
    const wrapper = await openPanel({
      locations: [
        locFixture(),
        locFixture({ id: 2, name: "架上", sort: 1, enabled: false }),
      ],
    });
    invokeMock.mockClear();

    await wrapper.findAll(".loc-row")[0]!.find(".loc-deactivate-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_location_enabled", { id: 1, enabled: false });
    expect(showSuccessMock).toHaveBeenLastCalledWith("已停用「阳台」");

    await wrapper.findAll(".loc-row")[1]!.find(".loc-activate-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_location_enabled", { id: 2, enabled: true });
    expect(showSuccessMock).toHaveBeenLastCalledWith("已启用「架上」");
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("行级停用失败弹失败轻提示带原因", async () => {
    const wrapper = await openPanel({ enableError: "库被锁住" });

    await wrapper.find(".loc-deactivate-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("停用失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
  });

  it("行级删除成功弹「已删除」并移行、抛 changed", async () => {
    const wrapper = await openPanel();

    await wrapper.find(".loc-erase-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("erase_location", { id: 1 });
    expect(wrapper.findAll(".loc-row").length).toBe(1);
    expect(showSuccessMock).toHaveBeenCalledWith("已删除「阳台」");
    expect(wrapper.emitted("changed")).toBeTruthy();
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("行级删除失败（被窝引用）弹失败轻提示带原因，行保留、内联上下文照旧", async () => {
    const wrapper = await openPanel({
      eraseError: "该地点仍被 1 个窝使用，不能删除；可改为停用",
    });

    await wrapper.find(".loc-erase-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith(
      "删除失败",
      "该地点仍被 1 个窝使用，不能删除；可改为停用",
    );
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.findAll(".loc-row").length).toBe(2);
    expect(wrapper.find(".loc-error").text()).toContain("改为停用");
  });
});
