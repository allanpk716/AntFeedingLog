import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import SpeciesManagerPanel from "./SpeciesManagerPanel.vue";
import type { CustomSpecies } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的 invokeMock）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

// 轻提示接线（species-profile 票 05）：toast 模块整体 mock 掉，断言调用点；
// store/宿主自身行为在 toast.test.ts / ToastHost.test.ts
const { showErrorMock, showSuccessMock } = vi.hoisted(() => ({
  showErrorMock: vi.fn(),
  showSuccessMock: vi.fn(),
}));
vi.mock("../lib/toast", () => ({
  showSuccess: showSuccessMock,
  showError: showErrorMock,
}));

/** 造一行自建物种（真实 IPC 形状，Rust species::CustomSpecies）。 */
function customRow(
  overrides: Partial<CustomSpecies> & { id: number; name: string },
): CustomSpecies {
  return {
    key: `custom-${overrides.id}`,
    type: "自定义",
    created_at: "2026-09-29 10:00:00",
    referenced: false,
    ...overrides,
  };
}

interface Extra {
  rows?: CustomSpecies[];
  listError?: string;
  renameError?: string;
  deleteError?: string;
}

async function openPanel(extra: Extra = {}): Promise<VueWrapper> {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_custom_species":
        if (extra.listError) throw extra.listError;
        return (
          extra.rows ?? [
            customRow({ id: 1, name: "蜜罐蚁" }),
            customRow({ id: 2, name: "收获蚁自建", type: "收获蚁", referenced: true }),
          ]
        );
      case "rename_custom_species":
        if (extra.renameError) throw extra.renameError;
        return customRow({ id: 1, name: "蜜罐蚁二号" });
      case "delete_custom_species":
        if (extra.deleteError) throw extra.deleteError;
        return null;
      default:
        return null;
    }
  });
  const wrapper = mount(SpeciesManagerPanel);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  showSuccessMock.mockClear();
  showErrorMock.mockClear();
});

describe("自建物种管理面板·清单渲染（species-profile 票 05）", () => {
  it("渲染自建物种清单：名字回显、类型、被引用标记各行可见", async () => {
    const wrapper = await openPanel();

    const rows = wrapper.findAll(".sp-row");
    expect(rows.length).toBe(2);
    expect((rows[0]!.find(".sp-name-input").element as HTMLInputElement).value).toBe("蜜罐蚁");
    expect(rows[0]!.find(".sp-type").text()).toBe("自定义");
    expect(rows[1]!.find(".sp-name-input").element as HTMLInputElement).toBeTruthy();
    expect(rows[1]!.find(".sp-type").text()).toBe("收获蚁");
    // 被引用标记只在被引用行出现
    expect(rows[0]!.find(".sp-referenced-chip").exists()).toBe(false);
    expect(rows[1]!.find(".sp-referenced-chip").exists()).toBe(true);
  });

  it("无自建物种 → 空态提示，不渲染行", async () => {
    const wrapper = await openPanel({ rows: [] });

    expect(wrapper.findAll(".sp-row").length).toBe(0);
    expect(wrapper.find(".sp-empty").exists()).toBe(true);
    expect(wrapper.find(".sp-empty").text()).toContain("自建物种");
  });

  it("清单读取挂起期间显示全局 LoadingHint 占位，拉到即摘", async () => {
    let release!: (rows: CustomSpecies[]) => void;
    invokeMock.mockImplementation(
      (cmd: string) =>
        new Promise<CustomSpecies[]>((resolve) => {
          if (cmd === "list_custom_species") release = resolve;
        }),
    );
    const wrapper = mount(SpeciesManagerPanel);

    expect(wrapper.find(".loading-hint").exists()).toBe(true);
    expect(wrapper.findAll(".sp-row").length).toBe(0);

    release([customRow({ id: 1, name: "蜜罐蚁" })]);
    await flushPromises();
    expect(wrapper.find(".loading-hint").exists()).toBe(false);
    expect(wrapper.findAll(".sp-row").length).toBe(1);
    wrapper.unmount();
  });

  it("清单读取失败 → 显示读取失败与原因，提供重试；重试成功恢复渲染", async () => {
    const wrapper = await openPanel({ listError: "库打不开" });

    expect(wrapper.find(".sp-load-error").text()).toContain("读取失败");
    expect(wrapper.find(".sp-load-error").text()).toContain("库打不开");

    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_custom_species" ? [customRow({ id: 1, name: "蜜罐蚁" })] : null,
    );
    await wrapper.find(".sp-reload-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".sp-load-error").exists()).toBe(false);
    expect(wrapper.findAll(".sp-row").length).toBe(1);
  });
});

describe("自建物种管理面板·改名（species-profile 票 05）", () => {
  it("改名保存生效：入参带 key 与 trim 后新名，成功轻提示，重拉清单并抛 changed", async () => {
    const wrapper = await openPanel();
    await wrapper.findAll(".sp-row")[0]!.find(".sp-name-input").setValue("  蜜罐蚁二号  ");

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_custom_species":
          return [customRow({ id: 1, name: "蜜罐蚁二号" })];
        case "rename_custom_species":
          return customRow({ id: 1, name: "蜜罐蚁二号" });
        default:
          return null;
      }
    });
    await wrapper.findAll(".sp-row")[0]!.find(".sp-rename-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("rename_custom_species", {
      key: "custom-1",
      newName: "蜜罐蚁二号",
    });
    expect(showSuccessMock).toHaveBeenCalledWith("已改名为「蜜罐蚁二号」");
    // 前端只需重新拉取（级联刷新引用窝快照在后端）
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_custom_species")).toBe(true);
    expect(wrapper.emitted("changed")).toBeTruthy();
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("重名报错走失败 toast（带原因）：成功提示不弹、行保留、不抛 changed", async () => {
    const wrapper = await openPanel({
      renameError: "自建物种名字「蜜罐蚁二号」已存在",
    });
    await wrapper.findAll(".sp-row")[0]!.find(".sp-name-input").setValue("蜜罐蚁二号");

    await wrapper.findAll(".sp-row")[0]!.find(".sp-rename-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith(
      "改名失败",
      "自建物种名字「蜜罐蚁二号」已存在",
    );
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.emitted("changed")).toBeFalsy();
    // 失败后行保留，输入框仍在
    expect(wrapper.findAll(".sp-row").length).toBe(2);
  });

  it("名字未改或空白 → 改名按钮禁用，不产生 rename 调用", async () => {
    const wrapper = await openPanel();

    // 未改：禁用
    expect((wrapper.findAll(".sp-row")[0]!.find(".sp-rename-btn").element as HTMLButtonElement).disabled).toBe(true);
    // 清空：禁用（空名不发请求）
    await wrapper.findAll(".sp-row")[0]!.find(".sp-name-input").setValue("   ");
    expect((wrapper.findAll(".sp-row")[0]!.find(".sp-rename-btn").element as HTMLButtonElement).disabled).toBe(true);

    invokeMock.mockClear();
    await wrapper.findAll(".sp-row")[0]!.find(".sp-rename-btn").trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "rename_custom_species")).toBe(false);
  });
});

describe("自建物种管理面板·删除（species-profile 票 05）", () => {
  it("未引用行可删：走 delete_custom_species，成功轻提示并抛 changed、重拉清单", async () => {
    const wrapper = await openPanel();

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_custom_species":
          return [customRow({ id: 2, name: "收获蚁自建", type: "收获蚁", referenced: true })];
        case "delete_custom_species":
          return null;
        default:
          return null;
      }
    });
    await wrapper.findAll(".sp-row")[0]!.find(".sp-erase-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("delete_custom_species", { key: "custom-1" });
    expect(showSuccessMock).toHaveBeenCalledWith("已删除「蜜罐蚁」");
    expect(wrapper.emitted("changed")).toBeTruthy();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "list_custom_species")).toBe(true);
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("被引用行删除禁用并提示原因；点击不产生删除调用", async () => {
    const wrapper = await openPanel();

    const erase = wrapper.findAll(".sp-row")[1]!.find(".sp-erase-btn");
    expect((erase.element as HTMLButtonElement).disabled).toBe(true);
    expect(erase.attributes("title")).toContain("不能删除");

    invokeMock.mockClear();
    await erase.trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "delete_custom_species")).toBe(false);
  });

  it("删除失败：失败轻提示带原因，行保留、不抛 changed", async () => {
    const wrapper = await openPanel({ deleteError: "库被锁住" });

    await wrapper.findAll(".sp-row")[0]!.find(".sp-erase-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("删除失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.emitted("changed")).toBeFalsy();
    expect(wrapper.findAll(".sp-row").length).toBe(2);
  });
});
