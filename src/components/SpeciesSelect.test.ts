import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import SpeciesSelect from "./SpeciesSelect.vue";
import { getSpeciesProfile, allSpeciesProfiles, speciesSummary } from "../lib/speciesProfiles";
import { clearToasts, toastItems } from "../lib/toast";
import type { CustomSpecies } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 ColonyFormDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

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

async function mountSel(
  modelValue = "",
  custom: CustomSpecies[] = [],
): Promise<VueWrapper> {
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === "list_custom_species" ? custom : null,
  );
  const w = mount(SpeciesSelect, { props: { modelValue } });
  await flushPromises();
  return w;
}

/** 点开面板（自建清单在此刻异步拉取）。 */
async function openPanel(w: VueWrapper): Promise<void> {
  await w.find(".species-trigger").trigger("click");
  await flushPromises();
}

/** 面板里的选项按钮文本清单（不含「不指定」；只取选项名，「自建」小标不算）。 */
function optionTexts(w: VueWrapper): string[] {
  return w
    .findAll(".option")
    .filter((o) => !o.classes().includes("none-option"))
    .map((o) => o.find(".option-label").text());
}

function optionOf(w: VueWrapper, label: string) {
  const opt = w
    .findAll(".option")
    .find((o) => !o.classes().includes("none-option") && o.text().includes(label));
  expect(opt, `应有「${label}」选项`).toBeDefined();
  return opt!;
}

beforeEach(() => {
  invokeMock.mockReset();
  clearToasts();
});

afterEach(() => {
  clearToasts();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("SpeciesSelect 触发器与选中回显", () => {
  it("未指定显示占位文案（物种可不填）；面板里有「不指定」项，点击回传空串并收面板", async () => {
    const w = await mountSel("messor-barbarus");
    expect(w.find(".species-trigger").text()).toContain("红头收获蚁");

    await openPanel(w);
    await w.find(".none-option").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual([""]);
    expect(w.find(".species-panel").exists()).toBe(false);

    const w2 = await mountSel("");
    expect(w2.find(".species-trigger").text()).toContain("未指定");
  });

  it("选中内置物种按 key 显示当前中文名；自建 key 经自建清单回显名字；不可解析原样显示 key 兜底", async () => {
    const w = await mountSel(
      "custom-2",
      [customRow({ id: 1, name: "蜜罐蚁" }), customRow({ id: 2, name: "阿根廷蚁" })],
    );
    expect(w.find(".species-trigger").text()).toContain("阿根廷蚁");

    const w2 = await mountSel("messor-aciculatus");
    expect(w2.find(".species-trigger").text()).toContain("针毛收获蚁");

    const w3 = await mountSel("gone-key");
    expect(w3.find(".species-trigger").text()).toContain("gone-key");
  });
});

describe("SpeciesSelect 类型分组（档案 distinct + 自建类型，动态）", () => {
  it("内置按 distinct 动态成组（弓背蚁/收获蚁），自建类型追加成组、同名类型并入既有组", async () => {
    const w = await mountSel("", [
      customRow({ id: 1, name: "蜜罐蚁" }),
      customRow({ id: 2, name: "收获蚁X", type: "收获蚁" }),
    ]);
    await openPanel(w);

    const labels = w.findAll(".group-label").map((g) => g.text());
    expect(labels).toEqual(["弓背蚁", "收获蚁", "自定义"]);

    // 自建「收获蚁X」并入既有收获蚁组；蜜罐蚁落在追加的「自定义」组
    const groups = w.findAll(".species-group");
    const harvest = groups.find((g) => g.find(".group-label").text() === "收获蚁")!;
    expect(harvest.findAll(".option").map((o) => o.text()).join("|")).toContain("收获蚁X");
    const customGroup = groups.find((g) => g.find(".group-label").text() === "自定义")!;
    expect(customGroup.findAll(".option").map((o) => o.text()).join("|")).toContain("蜜罐蚁");
  });

  it("自建选项带「自建」小标，与内置区分", async () => {
    const w = await mountSel("", [customRow({ id: 1, name: "蜜罐蚁" })]);
    await openPanel(w);
    expect(optionOf(w, "蜜罐蚁").find(".custom-tag").text()).toBe("自建");
  });
});

describe("SpeciesSelect 冬眠需求筛选（三值）", () => {
  it("选「冬眠」只剩冬眠档案（浅冬眠/不冬眠消失）；自建无档案数据不受筛选、恒显示", async () => {
    const w = await mountSel("", [customRow({ id: 1, name: "蜜罐蚁" })]);
    await openPanel(w);

    await w.find(".hib-select").setValue("冬眠");
    await flushPromises();

    const texts = optionTexts(w);
    expect(texts).toContain("蜜罐蚁"); // 自建恒显示
    expect(texts).toContain("日本弓背蚁"); // 冬眠
    expect(texts).not.toContain("针毛收获蚁"); // 浅冬眠
    // 动态核对：冬眠档案一个不少
    for (const name of allSpeciesProfiles()
      .filter((p) => p.hibernation === "冬眠")
      .map((p) => p.cnName)) {
      expect(texts).toContain(name);
    }

    // 清回「全部」恢复
    await w.find(".hib-select").setValue("");
    await flushPromises();
    expect(optionTexts(w)).toContain("针毛收获蚁");
  });
});

describe("SpeciesSelect 搜索（中文名/拉丁名/别名 + 自建名字）", () => {
  const row = [customRow({ id: 1, name: "蜜罐蚁" })];

  it("中文名命中", async () => {
    const w = await mountSel("", row);
    await openPanel(w);
    await w.find(".species-search").setValue("针毛收获蚁");
    expect(optionTexts(w)).toEqual(["针毛收获蚁"]);
  });

  it("拉丁名命中（大小写不敏感）", async () => {
    const w = await mountSel("", row);
    await openPanel(w);
    await w.find(".species-search").setValue("japonicus");
    expect(optionTexts(w)).toEqual(["日本弓背蚁"]);
  });

  it("别名命中", async () => {
    const w = await mountSel("", row);
    await openPanel(w);
    await w.find(".species-search").setValue("巴巴拉");
    expect(optionTexts(w)).toEqual(["红头收获蚁"]);
  });

  it("自建名字命中；全部搜空时给空态提示", async () => {
    const w = await mountSel("", row);
    await openPanel(w);
    await w.find(".species-search").setValue("蜜罐");
    expect(optionTexts(w)).toEqual(["蜜罐蚁"]);

    await w.find(".species-search").setValue("不存在的物种xyz");
    expect(optionTexts(w)).toEqual([]);
    expect(w.find(".no-match").exists()).toBe(true);
  });
});

describe("SpeciesSelect 悬停一句话摘要（票 01 摘要助手）", () => {
  it("内置选项 title = 温度/食物/冬眠三要素（缺项「暂无资料」）；自建无 title", async () => {
    const w = await mountSel("", [customRow({ id: 1, name: "蜜罐蚁" })]);
    await openPanel(w);

    const p = getSpeciesProfile("messor-aciculatus")!;
    expect(optionOf(w, "针毛收获蚁").attributes("title")).toBe(speciesSummary(p));
    expect(optionOf(w, "蜜罐蚁").attributes("title")).toBeUndefined();
  });
});

describe("SpeciesSelect 选项选择（保存链路的源头）", () => {
  it("点内置/自建选项 → update:modelValue 传 key 并收面板", async () => {
    const w = await mountSel("", [customRow({ id: 1, name: "蜜罐蚁" })]);
    await openPanel(w);

    await optionOf(w, "针毛收获蚁").trigger("click");
    expect(w.emitted("update:modelValue")![0]).toEqual(["messor-aciculatus"]);
    expect(w.find(".species-panel").exists()).toBe(false);

    await openPanel(w);
    await optionOf(w, "蜜罐蚁").trigger("click");
    expect(w.emitted("update:modelValue")![1]).toEqual(["custom-1"]);
  });

  it("选中项在面板里高亮（picked）", async () => {
    const w = await mountSel("messor-barbarus");
    await openPanel(w);
    expect(optionOf(w, "红头收获蚁").classes()).toContain("picked");
  });
});

describe("SpeciesSelect ＋自建物种内联新建（桌面专属）", () => {
  // 写命令桌面专属：本组用例全程以桌面环境挂载
  beforeEach(() => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  });

  it("建后即选：回传新 key、清单即时追加、成功轻提示", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_custom_species") return [customRow({ id: 1, name: "阿根廷蚁" })];
      if (cmd === "create_custom_species") return customRow({ id: 2, name: "蜜罐蚁" });
      return null;
    });
    const w = mount(SpeciesSelect, { props: { modelValue: "" } });
    await flushPromises();
    await openPanel(w);
    await w.find(".create-toggle").trigger("click");

    await w.find("input.create-name").setValue("蜜罐蚁");
    await w.find(".create-submit").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("create_custom_species", {
      name: "蜜罐蚁",
      speciesType: "自定义",
    });
    expect(w.emitted("update:modelValue")![0]).toEqual(["custom-2"]);
    // 面板收起；父级 v-model 接住后触发器即显新选（新建表单随面板一起收）
    await w.setProps({ modelValue: "custom-2" });
    expect(w.find(".species-panel").exists()).toBe(false);
    expect(w.find(".species-trigger").text()).toContain("蜜罐蚁");
    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("success");
    expect(last?.message).toContain("蜜罐蚁");
  });

  it("类型默认「自定义」、可改填新类型名（随创建入参上送）", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "create_custom_species" ? customRow({ id: 5, name: "蜜罐蚁" }) : [],
    );
    const w = mount(SpeciesSelect, { props: { modelValue: "" } });
    await flushPromises();
    await openPanel(w);
    await w.find(".create-toggle").trigger("click");

    expect((w.find("input.create-type").element as HTMLInputElement).value).toBe("自定义");
    await w.find("input.create-name").setValue("蜜罐蚁");
    await w.find("input.create-type").setValue("蜜罐蚁科");
    await w.find(".create-submit").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("create_custom_species", {
      name: "蜜罐蚁",
      speciesType: "蜜罐蚁科",
    });
  });

  it("名字留空是字段校验：内联红字拦截、不发命令、不进轻提示", async () => {
    const w = await mountSel();
    await openPanel(w);
    await w.find(".create-toggle").trigger("click");

    await w.find("input.create-name").setValue("   ");
    await w.find(".create-submit").trigger("click");

    expect(w.find(".create-error").text()).toContain("不能为空");
    expect(invokeMock).not.toHaveBeenCalledWith("create_custom_species", expect.anything());
    expect(toastItems.value).toHaveLength(0);
  });

  it("重名：提示已存在并直接选用既有行，不重复建行", async () => {
    const existing = customRow({ id: 1, name: "蜜罐蚁", referenced: true });
    const w = await mountSel("", [existing]);
    await openPanel(w);
    await w.find(".create-toggle").trigger("click");

    await w.find("input.create-name").setValue(" 蜜罐蚁 ");
    await w.find(".create-submit").trigger("click");
    await flushPromises();

    expect(invokeMock).not.toHaveBeenCalledWith("create_custom_species", expect.anything());
    expect(w.emitted("update:modelValue")![0]).toEqual(["custom-1"]);
    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("success");
    expect(last?.message).toContain("已存在");
  });

  it("创建失败：红色轻提示带原因，不选用不收面板", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "create_custom_species") throw "库已锁";
      return [];
    });
    const w = mount(SpeciesSelect, { props: { modelValue: "" } });
    await flushPromises();
    await openPanel(w);
    await w.find(".create-toggle").trigger("click");
    await w.find("input.create-name").setValue("蜜罐蚁");
    await w.find(".create-submit").trigger("click");
    await flushPromises();

    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("error");
    expect(last?.message).toBe("自建物种创建失败");
    expect(last?.reason).toBe("库已锁");
    expect(w.emitted("update:modelValue")).toBeUndefined();
  });
});

describe("SpeciesSelect 网页端（非 Tauri）：清单只读、无内联新建入口", () => {
  it("无「＋自建物种」入口；自建清单照常展示可选", async () => {
    const w = await mountSel("", [customRow({ id: 1, name: "蜜罐蚁" })]);
    await openPanel(w);
    expect(w.find(".create-toggle").exists()).toBe(false);
    expect(optionTexts(w)).toContain("蜜罐蚁");
  });

  it("桌面端入口照常渲染（环境判定的另一面）", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const w = await mountSel();
    await openPanel(w);
    expect(w.find(".create-toggle").exists()).toBe(true);
  });
});

describe("SpeciesSelect 自建清单加载态（全局 LoadingHint 规范）", () => {
  it("首次打开面板：异步拉取期间显示「加载中…」，拉到即摘；清单失败降级为失败说明，内置照常可选", async () => {
    let release!: (rows: CustomSpecies[]) => void;
    invokeMock.mockImplementation(
      (cmd: string) =>
        new Promise<CustomSpecies[]>((resolve) => {
          if (cmd === "list_custom_species") release = resolve;
        }),
    );
    const w = mount(SpeciesSelect, { props: { modelValue: "" } });
    await w.find(".species-trigger").trigger("click");

    // 内置档案在内存，加载中已即时可选；自建区显示全局加载占位
    expect(optionTexts(w)).toContain("针毛收获蚁");
    expect(w.text()).toContain("加载中…");

    release([customRow({ id: 1, name: "蜜罐蚁" })]);
    await flushPromises();
    expect(w.text()).not.toContain("加载中…");
    expect(optionTexts(w)).toContain("蜜罐蚁");

    // 失败路径：另一实例首次打开 → 失败说明 + 内置照常
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_custom_species") throw "HTTP 500";
      return null;
    });
    const w2 = mount(SpeciesSelect, { props: { modelValue: "" } });
    await openPanel(w2);
    expect(w2.find(".custom-failed").exists()).toBe(true);
    expect(optionTexts(w2)).toContain("针毛收获蚁");
  });

  it("同一面板重开：清单已在内存，即时渲染不进加载态", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_custom_species" ? [customRow({ id: 1, name: "蜜罐蚁" })] : null,
    );
    const w = mount(SpeciesSelect, { props: { modelValue: "" } });
    await openPanel(w);
    await w.find(".species-trigger").trigger("click"); // 收
    invokeMock.mockClear();
    await openPanel(w); // 重开
    expect(invokeMock).not.toHaveBeenCalled(); // 清单走内存，不重拉
    expect(w.text()).not.toContain("加载中…");
    expect(optionTexts(w)).toContain("蜜罐蚁");
  });
});
