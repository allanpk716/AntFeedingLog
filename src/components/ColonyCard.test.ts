import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type DOMWrapper } from "@vue/test-utils";
import ColonyCard from "./ColonyCard.vue";
import type { Colony, ColonyAction, NestPhotoMeta } from "../types";
// 拍一张（checkin-photo-entry 票 02）：轻提示走全局唯一 toast 模块，断言真实状态
import { clearToasts, toastItems } from "../lib/toast";
import { todayIso } from "../lib/dates";
import { getSpeciesProfile } from "../lib/speciesProfiles";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 QuickLogDialog.test.ts 先例）
const { invokeMock, loadPhotoBlobUrlMock, revokeObjectUrlMock, createCheckinPhotosHttpMock } =
  vi.hoisted(() => ({
    invokeMock: vi.fn(),
    loadPhotoBlobUrlMock: vi.fn(),
    revokeObjectUrlMock: vi.fn(),
    createCheckinPhotosHttpMock: vi.fn(),
  }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});
// 窝头像票 02：网页 blob 取图/释放打桩（沿 NestCheckinDialog.test.ts 先例）；
// 拍一张票 02：网页创建模式上传打桩（编排断言用），其余透传
vi.mock("../lib/photos", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/photos")>();
  return {
    ...actual,
    loadPhotoBlobUrl: loadPhotoBlobUrlMock,
    revokeObjectUrl: revokeObjectUrlMock,
    createCheckinPhotosHttp: createCheckinPhotosHttpMock,
  };
});

/** 覆盖 0.5.0 撑爆单行格的两个载荷：周期后缀 + 仅登记标签的长名操作 */
const actions: ColonyAction[] = [
  {
    action_id: 4, name: "垃圾清理", icon: "🗑", kind: "reminding", is_feeding: false,
    suggested_interval_days: null, interval_from_colony: true, effective_interval_days: 7,
    days_since_last: 3, overdue: false, foods: [],
  },
  {
    action_id: 2, name: "活动区换水", icon: "💧", kind: "log_only", is_feeding: false,
    suggested_interval_days: null, days_since_last: 2, overdue: false, foods: [],
  },
];

const colony: Colony = {
  id: 1, name: "大头一号", species: "大头收获蚁", location_id: null, start_date: "2026-01-20",
  status: "active", days_raised: 241, actions, recent: [], hibernation: null,
  checkin: { latest: null, baseline_date: null, days_since_last: null },
};

/** NestCheckinDialog 打桩（不依赖其内部 DOM）：断言「点头像 = 打开巢况时间线弹窗」 */
const CheckinDialogStub = { template: '<div data-testid="checkin-dialog-stub"></div>' };

function mountCard(c: Colony = colony, extraProps: Record<string, unknown> = {}) {
  return mount(ColonyCard, {
    props: { colony: c, ...extraProps },
    global: { stubs: { NestCheckinDialog: CheckinDialogStub } },
  });
}

describe("ColonyCard 操作块两行结构（mock-d 变体 A 决议）", () => {
  it("上行 .t-top 装图标/名字/仅登记标签，状态药丸独立下行不与名字同行", () => {
    const w = mountCard();
    const trash = w.find('.tile[data-action-id="4"]');
    const top = trash.find(".t-top");
    expect(top.exists()).toBe(true);
    expect(top.find(".t-ico").text()).toBe("🗑");
    expect(top.find(".t-name").text()).toBe("垃圾清理");
    // 药丸是 .t-top 的兄弟而非子级——两行结构的落点
    expect(trash.find(".pill").element.parentElement?.classList.contains("t-top")).toBe(false);
    expect(trash.find(".pill").text()).toBe("距上次 3 天 / 周期 7 天");
    // 「仅登记」标签仍随名字在上行（挤不下折行交给 CSS flex-wrap）
    const water = w.find('.tile[data-action-id="2"]');
    expect(water.find(".t-top .t-tag").text()).toBe("仅登记");
  });
});

// ── 窝卡片头像（窝头像票 02）：占位 / 取图双通路 / 裁剪定位 / 点击开时间线 ──

/** 头像照片夹具：横图裁剪例用（crop 语义见组件内注释：size 按长边、x/y 各按宽高归一化） */
const avatarPhoto: NestPhotoMeta = {
  id: 901,
  checkin_id: 900,
  rel_path: "1/abc.jpg",
  original_name: null,
  note: "",
  crop: { x: 0.25, y: 0, size: 0.5 },
};

const colonyWithAvatar: Colony = { ...colony, avatar: avatarPhoto };

/** happy-dom 不加载真实图片：手工注入 naturalWidth/Height 并触发 load 事件 */
async function loadImgNaturalSize(img: DOMWrapper<Element>, w: number, h: number) {
  Object.defineProperty(img.element, "naturalWidth", { value: w, configurable: true });
  Object.defineProperty(img.element, "naturalHeight", { value: h, configurable: true });
  await img.trigger("load");
}

/** 内联样式百分比取数（裁剪定位断言用） */
function stylePct(img: DOMWrapper<Element>, prop: string): number {
  return parseFloat((img.element as HTMLElement).style.getPropertyValue(prop));
}

describe("窝卡片头像（窝头像票 02）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    loadPhotoBlobUrlMock.mockReset();
    revokeObjectUrlMock.mockReset();
  });

  it("无照片窝（avatar 缺省/null）显示 🐜 占位，不渲染 img、不发取图请求", () => {
    for (const c of [colony, { ...colony, avatar: null }]) {
      const w = mountCard(c);
      const ph = w.find('[data-testid="avatar-ph"]');
      expect(ph.exists()).toBe(true);
      expect(ph.text()).toContain("🐜");
      expect(w.find("img.avatar-img").exists()).toBe(false);
      expect(loadPhotoBlobUrlMock).not.toHaveBeenCalled();
      w.unmount();
    }
  });

  it("网页端头像走 loadPhotoBlobUrl 的 objectURL", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:ok-1");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledTimes(1);
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledWith("1/abc.jpg");
    const img = w.find("img.avatar-img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("blob:ok-1");
    w.unmount();
  });

  it("网页端取图失败（reject）回退 🐜 占位，不崩溃", async () => {
    loadPhotoBlobUrlMock.mockRejectedValue("HTTP 404");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    expect(w.find('[data-testid="avatar-ph"]').exists()).toBe(true);
    expect(w.find("img.avatar-img").exists()).toBe(false);
  });

  it("图片加载失败（img error 事件，文件缺失）回退占位", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:broken");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    await w.find("img.avatar-img").trigger("error");
    expect(w.find('[data-testid="avatar-ph"]').exists()).toBe(true);
    expect(w.find("img.avatar-img").exists()).toBe(false);
  });

  it("组件卸载释放网页端 blob URL", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:bye");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    w.unmount();
    expect(revokeObjectUrlMock).toHaveBeenCalledWith("blob:bye");
  });

  it("头像照片变了：释放旧 blob、按新 rel_path 取图", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:old");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    expect(w.find("img.avatar-img").attributes("src")).toBe("blob:old");
    loadPhotoBlobUrlMock.mockResolvedValue("blob:new");
    await w.setProps({
      colony: { ...colonyWithAvatar, avatar: { ...avatarPhoto, rel_path: "2/b.jpg" } },
    });
    await flushPromises();
    expect(loadPhotoBlobUrlMock).toHaveBeenLastCalledWith("2/b.jpg");
    expect(w.find("img.avatar-img").attributes("src")).toBe("blob:new");
    expect(revokeObjectUrlMock).toHaveBeenCalledWith("blob:old");
  });

  it("裁剪定位（横图例）：200×100 + crop{x:0.25,y:0,size:0.5} → 放大 2 倍左移 50%，裁剪区铺满方框", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:ok-1");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    const img = w.find("img.avatar-img");
    await loadImgNaturalSize(img, 200, 100);
    // side = size·长边 = 100px：img 铺 200%×100%，左移 x·宽/side = 50%，y=0 不移
    expect(stylePct(img, "width")).toBeCloseTo(200);
    expect(stylePct(img, "height")).toBeCloseTo(100);
    expect(stylePct(img, "left")).toBeCloseTo(-50);
    expect(stylePct(img, "top")).toBeCloseTo(0);
  });

  it("裁剪定位（竖图例）：crop 空缺 = 默认居中（100×200 取中心最大方形）", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:ok-1");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    // 换成无 crop 的头像
    await w.setProps({
      colony: { ...colonyWithAvatar, avatar: { ...avatarPhoto, crop: null } },
    });
    await flushPromises();
    const img = w.find("img.avatar-img");
    await loadImgNaturalSize(img, 100, 200);
    // 中心最大方形 = 宽 100px：img 铺 100%×200%，x=0 不移，上移 (1-宽/高)/2·高/side = 50%
    expect(stylePct(img, "width")).toBeCloseTo(100);
    expect(stylePct(img, "height")).toBeCloseTo(200);
    expect(stylePct(img, "left")).toBeCloseTo(0);
    expect(stylePct(img, "top")).toBeCloseTo(-50);
  });

  it("点横幅或「巢况」按钮都打开巢况时间线弹窗（banner 上的两入口同一弹窗）", async () => {
    const byAvatar = mountCard(colonyWithAvatar);
    await byAvatar.find('[data-testid="colony-avatar"]').trigger("click");
    expect(byAvatar.find('[data-testid="checkin-dialog-stub"]').exists()).toBe(true);
    byAvatar.unmount();

    const byButton = mountCard(colonyWithAvatar);
    await byButton.find(".bact-btn").trigger("click");
    expect(byButton.find('[data-testid="checkin-dialog-stub"]').exists()).toBe(true);
  });

  it("横幅信息层：名字/物种徽章/状态徽章/天数叠在照片下沿（avatar-banner 乙-2）", () => {
    const w = mountCard(colonyWithAvatar);
    expect(w.find(".banner-img .bname").text()).toBe("大头一号");
    expect(w.find(".banner-img .bchip").text()).toBe("大头收获蚁");
    expect(w.findAll(".banner-img .bchip")[1].text()).toContain("活跃");
    expect(w.find(".banner-img .bdays .n").text()).toBe("241");
  });

  describe("桌面 asset 通路", () => {
    afterEach(() => {
      delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    });

    it("头像 src = photoSrc(rel_path, get_photo_abs_dir 结果)，不抛错", async () => {
      (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
        convertFileSrc: (p: string) => p,
      };
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === "get_photo_abs_dir" ? "C:/data/photos" : undefined,
      );
      const w = mountCard(colonyWithAvatar);
      await flushPromises();
      const img = w.find("img.avatar-img");
      expect(img.exists()).toBe(true);
      expect(img.attributes("src")).toBe("C:/data/photos/1/abc.jpg");
      expect(loadPhotoBlobUrlMock).not.toHaveBeenCalled();
    });
  });
});

// ── 物种徽章（species-profile 票 03）：key 实时解析优先/快照兜底/两皆空/悬停 ──

describe("物种徽章（species-profile 票 03）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    loadPhotoBlobUrlMock.mockReset();
    revokeObjectUrlMock.mockReset();
  });

  it("解析优先：内置 key 显示当前中文名，不显示旧快照；悬停=拉丁名 · 类型 · 冬眠需求", () => {
    const w = mountCard({ ...colony, species: "旧快照文本", species_key: "messor-barbarus" });
    const badge = w.find('[data-testid="species-badge"]');
    expect(badge.exists()).toBe(true);
    expect(badge.text()).toBe("红头收获蚁");
    const p = getSpeciesProfile("messor-barbarus")!;
    expect(badge.attributes("title")).toBe(`${p.latinName} · ${p.type} · ${p.hibernation}`);
  });

  it("自建 key 经自建清单解析：显示当前名，悬停=名字 · 类型", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_custom_species"
        ? [
            {
              id: 3,
              key: "custom-3",
              name: "蜜罐蚁",
              type: "蜜罐蚁科",
              created_at: "2026-09-29 10:00:00",
              referenced: true,
            },
          ]
        : null,
    );
    const w = mountCard({ ...colony, species: "蜜罐蚁", species_key: "custom-3" });
    await flushPromises();

    const badge = w.find('[data-testid="species-badge"]');
    expect(badge.exists()).toBe(true);
    expect(badge.text()).toBe("蜜罐蚁");
    expect(badge.attributes("title")).toBe("蜜罐蚁 · 蜜罐蚁科");
  });

  it("快照兜底：key 不可解析（自建清单查无/未知内置 key）显示快照列", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_custom_species" ? [] : null,
    );
    const w = mountCard({ ...colony, species: "旧文本", species_key: "custom-99" });
    await flushPromises();
    expect(w.find('[data-testid="species-badge"]').text()).toBe("旧文本");

    const w2 = mountCard({ ...colony, species: "未来蚂蚁", species_key: "future-ant" });
    expect(w2.find('[data-testid="species-badge"]').text()).toBe("未来蚂蚁");
  });

  it("两皆空：不渲染物种徽章，状态徽章保留", () => {
    const w = mountCard({ ...colony, species: null, species_key: null });
    expect(w.find('[data-testid="species-badge"]').exists()).toBe(false);
    expect(w.find(".bchip.st").exists()).toBe(true);
  });

  it("key 为空但快照在（旧数据）：按快照显示，不额外请求自建清单", () => {
    invokeMock.mockClear();
    const w = mountCard(colony);
    expect(w.find('[data-testid="species-badge"]').text()).toBe("大头收获蚁");
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

// ── 编辑入口双端可见（网页端窝编辑票 02）──

describe("编辑入口双端可见（网页端窝编辑票 02）", () => {
  afterEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("网页端（非 Tauri）：⋯菜单「✏️ 编辑窝信息」渲染，入口放开", () => {
    const w = mountCard();
    // 菜单容器 v-show 保 DOM（交互第三轮 #7），按钮断言不依赖菜单展开
    expect(w.find(".m-item.edit-btn").exists()).toBe(true);
  });

  it("桌面端：同样渲染，行为不变", () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const w = mountCard();
    expect(w.find(".m-item.edit-btn").exists()).toBe(true);
  });
});

// ── 「📷 拍一张」直达（checkin-photo-entry 票 02）：菜单置顶项 + 拍照编排 ──

describe("「📷 拍一张」菜单项与拍照编排（checkin-photo-entry 票 02）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    createCheckinPhotosHttpMock.mockReset();
    clearToasts();
  });

  afterEach(() => {
    clearToasts();
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("网页端：「📷 拍一张」是 ⋯ 菜单第一项（置顶）", () => {
    const w = mountCard();
    const items = w.findAll(".card-menu .m-item");
    expect(items.length).toBeGreaterThan(1);
    expect(items[0]!.text()).toContain("📷 拍一张");
  });

  it("桌面端：同样置顶渲染（双端可见）", () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const w = mountCard();
    const items = w.findAll(".card-menu .m-item");
    expect(items.length).toBeGreaterThan(1);
    expect(items[0]!.text()).toContain("📷 拍一张");
  });

  it("卡片层隐藏 capture input（网页端相机入口）：accept image/*、multiple、capture=environment", () => {
    const w = mountCard();
    const input = w.find("input.snap-file-input");
    expect(input.exists()).toBe(true);
    expect(input.attributes("accept")).toBe("image/*");
    expect(input.attributes("multiple")).toBeDefined();
    expect(input.attributes("capture")).toBe("environment");
  });

  it("网页端点菜单项：直调隐藏 capture input，不进弹窗", async () => {
    const w = mountCard();
    const inputEl = w.find("input.snap-file-input").element as HTMLInputElement;
    const clickSpy = vi.spyOn(inputEl, "click").mockImplementation(() => {});
    await w.find(".m-item.snap-btn").trigger("click");
    expect(clickSpy).toHaveBeenCalledTimes(1);
    // 一步直达的分界：不开巢况时间线弹窗
    expect(w.find('[data-testid="checkin-dialog-stub"]').exists()).toBe(false);
  });

  it("网页端编排：选好 File → 创建模式（colonyId+今天）→ 成功轻提示 + saved", async () => {
    createCheckinPhotosHttpMock.mockResolvedValue([]);
    const w = mountCard();
    const input = w.find("input.snap-file-input");
    const f = new File(["jpeg-bytes"], "snap.jpg", { type: "image/jpeg" });
    Object.defineProperty(input.element, "files", { value: [f], configurable: true });
    await input.trigger("change");
    await flushPromises();

    expect(createCheckinPhotosHttpMock).toHaveBeenCalledTimes(1);
    const [colonyId, files, date] = createCheckinPhotosHttpMock.mock.calls[0] as unknown as [
      number,
      File[],
      string,
    ];
    expect(colonyId).toBe(1);
    expect(Array.from(files)).toEqual([f]);
    expect(date).toBe(todayIso());

    // 无天然反馈的写操作（窗不关视图不变）：必须成功轻提示（全局规范），文案按规格
    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("success");
    expect(last?.message).toBe("✓ 已登记到「大头一号」· 头像已更新");
    // 抛 saved 让外层刷新（头像投影跟上）
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("网页端取消选照片（files 空）：不调通道、不抛 saved、无轻提示", async () => {
    const w = mountCard();
    const input = w.find("input.snap-file-input");
    Object.defineProperty(input.element, "files", { value: [], configurable: true });
    await input.trigger("change");
    await flushPromises();
    expect(createCheckinPhotosHttpMock).not.toHaveBeenCalled();
    expect(w.emitted("saved")).toBeUndefined();
    expect(toastItems.value).toHaveLength(0);
  });

  it("网页端失败：红色轻提示带原因，不抛 saved", async () => {
    createCheckinPhotosHttpMock.mockRejectedValue("HTTP 400");
    const w = mountCard();
    const input = w.find("input.snap-file-input");
    Object.defineProperty(input.element, "files", {
      value: [new File(["x"], "a.jpg")],
      configurable: true,
    });
    await input.trigger("change");
    await flushPromises();

    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("error");
    expect(last?.message).toBe("拍照登记失败");
    expect(last?.reason).toBe("HTTP 400");
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("桌面编排：pick_photo_files → save_checkin_with_photos（camelCase 路径数组）→ 成功轻提示 + saved", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "pick_photo_files" ? ["C:/pics/a.jpg", "C:/pics/b.jpg"] : undefined,
    );
    const w = mountCard();
    // 菜单 v-show 保 DOM：直接点按钮（menuAction 收菜单后执行）
    await w.find(".m-item.snap-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_checkin_with_photos", {
      colonyId: 1,
      photoPaths: ["C:/pics/a.jpg", "C:/pics/b.jpg"],
    });
    expect(createCheckinPhotosHttpMock).not.toHaveBeenCalled(); // 桌面不走 HTTP
    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("success");
    expect(last?.message).toBe("✓ 已登记到「大头一号」· 头像已更新");
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("桌面取消选文件：不调保存通道、不抛 saved、无轻提示", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "pick_photo_files" ? null : undefined,
    );
    const w = mountCard();
    await w.find(".m-item.snap-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).not.toHaveBeenCalledWith("save_checkin_with_photos", expect.anything());
    expect(w.emitted("saved")).toBeUndefined();
    expect(toastItems.value).toHaveLength(0);
  });

  it("桌面失败：红色轻提示带原因，不抛 saved", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "pick_photo_files") return ["C:/pics/a.jpg"];
      throw "库已锁";
    });
    const w = mountCard();
    await w.find(".m-item.snap-btn").trigger("click");
    await flushPromises();

    const last = toastItems.value[toastItems.value.length - 1];
    expect(last?.kind).toBe("error");
    expect(last?.message).toBe("拍照登记失败");
    expect(last?.reason).toBe("库已锁");
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("busy 期间「拍一张」菜单按钮禁用，完成后恢复", async () => {
    let release!: () => void;
    createCheckinPhotosHttpMock.mockImplementation(
      () => new Promise<void>((resolve) => (release = resolve)),
    );
    const w = mountCard();
    const input = w.find("input.snap-file-input");
    Object.defineProperty(input.element, "files", {
      value: [new File(["x"], "a.jpg")],
      configurable: true,
    });
    await input.trigger("change");
    await flushPromises();
    expect(w.find(".m-item.snap-btn").attributes("disabled")).toBeDefined();

    release();
    await flushPromises();
    expect(w.find(".m-item.snap-btn").attributes("disabled")).toBeUndefined();
  });
});

// ── 照片旋转联动（头像旋转：烧进文件）：弹窗旋转 → 卡片头像换新图 ──

describe("窝卡片头像旋转联动（头像旋转）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    loadPhotoBlobUrlMock.mockReset();
    revokeObjectUrlMock.mockReset();
  });

  afterEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("桌面：弹窗抛 rotatedPhoto → 头像 URL 加 ?v= 破缓存，连转递增", async () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
      convertFileSrc: (p: string) => `http://asset.localhost/${encodeURIComponent(p)}`,
    };
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "get_photo_abs_dir" ? "C:/photos" : null,
    );
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    const base = `http://asset.localhost/${encodeURIComponent("C:/photos/1/abc.jpg")}`;
    expect(w.find("img.avatar-img").attributes("src")).toBe(base);

    await w.find('[data-testid="colony-avatar"]').trigger("click"); // 打开巢况弹窗
    await w.findComponent(CheckinDialogStub).vm.$emit("rotatedPhoto", avatarPhoto);
    expect(w.find("img.avatar-img").attributes("src")).toBe(`${base}?v=1`);

    await w.findComponent(CheckinDialogStub).vm.$emit("rotatedPhoto", avatarPhoto);
    expect(w.find("img.avatar-img").attributes("src")).toBe(`${base}?v=2`);
  });

  it("浏览器：弹窗抛 rotatedPhoto → 释放旧 blob 重取新 blob", async () => {
    loadPhotoBlobUrlMock.mockImplementation(async () => "blob:old");
    const w = mountCard(colonyWithAvatar);
    await flushPromises();
    expect(w.find("img.avatar-img").attributes("src")).toBe("blob:old");

    await w.find('[data-testid="colony-avatar"]').trigger("click"); // 打开巢况弹窗
    loadPhotoBlobUrlMock.mockImplementation(async () => "blob:new");
    await w.findComponent(CheckinDialogStub).vm.$emit("rotatedPhoto", avatarPhoto);
    await flushPromises();

    expect(revokeObjectUrlMock).toHaveBeenCalledWith("blob:old");
    expect(w.find("img.avatar-img").attributes("src")).toBe("blob:new");
  });
});
