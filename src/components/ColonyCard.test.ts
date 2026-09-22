import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type DOMWrapper } from "@vue/test-utils";
import ColonyCard from "./ColonyCard.vue";
import type { Colony, ColonyAction, NestPhotoMeta } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 QuickLogDialog.test.ts 先例）
const { invokeMock, loadPhotoBlobUrlMock, revokeObjectUrlMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  loadPhotoBlobUrlMock: vi.fn(),
  revokeObjectUrlMock: vi.fn(),
}));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});
// 窝头像票 02：网页 blob 取图/释放打桩（沿 NestCheckinDialog.test.ts 先例），其余透传
vi.mock("../lib/photos", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/photos")>();
  return {
    ...actual,
    loadPhotoBlobUrl: loadPhotoBlobUrlMock,
    revokeObjectUrl: revokeObjectUrlMock,
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

  it("点头像打开巢况时间线弹窗（与「巢况」按钮同一弹窗）", async () => {
    const byAvatar = mountCard(colonyWithAvatar);
    await byAvatar.find('[data-testid="colony-avatar"]').trigger("click");
    expect(byAvatar.find('[data-testid="checkin-dialog-stub"]').exists()).toBe(true);
    byAvatar.unmount();

    const byButton = mountCard(colonyWithAvatar);
    await byButton.find(".checkin-btn").trigger("click");
    expect(byButton.find('[data-testid="checkin-dialog-stub"]').exists()).toBe(true);
  });

  it("显示形状默认圆形（shape 缺省 circle），传 square 渲染方形", () => {
    const w = mountCard(colonyWithAvatar);
    expect(w.find(".avatar").classes()).toContain("circle");
    const sq = mountCard(colonyWithAvatar, { shape: "square" });
    expect(sq.find(".avatar").classes()).toContain("square");
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
