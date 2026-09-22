import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import PhotoWallPage from "./PhotoWallPage.vue";
import ToastHost from "./ToastHost.vue";
import type { Colony, NestPhotoMeta, PhotoWallColony } from "../types";
import { clearToasts } from "../lib/toast";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 NestCheckinDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
// 网页端 blob 取图/释放打桩；桌面 photoSrc（convertFileSrc 需 Tauri 运行时）拼假 URL
const { loadPhotoBlobUrlMock, revokeObjectUrlMock } = vi.hoisted(() => ({
  loadPhotoBlobUrlMock: vi.fn(),
  revokeObjectUrlMock: vi.fn(),
}));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});
vi.mock("../lib/photos", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/photos")>();
  return {
    ...actual,
    photoSrc: (relPath: string, dir: string) => (dir ? `asset://photos/${relPath}` : ""),
    loadPhotoBlobUrl: loadPhotoBlobUrlMock,
    revokeObjectUrl: revokeObjectUrlMock,
  };
});

// jsdom/happy-dom 无 IntersectionObserver（或行为不定）：装可控假体，
// 用例里用 fire() 手动派发「进/出视口」事件
class FakeIntersectionObserver {
  static latest: FakeIntersectionObserver | null = null;
  private cb: IntersectionObserverCallback;
  readonly observed = new Set<Element>();
  constructor(cb: IntersectionObserverCallback) {
    this.cb = cb;
    FakeIntersectionObserver.latest = this;
  }
  observe(el: Element): void {
    this.observed.add(el);
  }
  unobserve(el: Element): void {
    this.observed.delete(el);
  }
  disconnect(): void {
    this.observed.clear();
  }
  /** 派发一批视口事件：match 命中的元素按 intersecting 给 isIntersecting。 */
  fire(match: (el: Element) => boolean, intersecting = true): void {
    const entries = [...this.observed]
      .filter(match)
      .map((el) => ({ target: el, isIntersecting: intersecting }) as unknown as IntersectionObserverEntry);
    this.cb(entries, this as unknown as IntersectionObserver);
  }
}

function fireThumb(photoId: number, intersecting = true): void {
  FakeIntersectionObserver.latest?.fire(
    (el) => el.getAttribute("data-photo-id") === String(photoId),
    intersecting,
  );
}

/** fireIn：对本页根内的全部已观察缩略图派发视口事件。 */
function fireIn(w: { find: (s: string) => { element: Element } }, intersecting = true): void {
  const rootEl = w.find(".photo-wall").element;
  FakeIntersectionObserver.latest?.fire((el) => rootEl.contains(el), intersecting);
}

// ── 载荷夹具：顺序即后端排序契约（窝内日期倒序、同日期登记创建序倒序、
//    登记内照片按上传序）——组件照载荷顺序渲染，绝不重排 ──

function photo(id: number, relPath: string): NestPhotoMeta {
  return { id, checkin_id: 0, rel_path: relPath, original_name: `p${id}.jpg`, note: "", crop: null };
}

const wallData: PhotoWallColony[] = [
  {
    colony_id: 1,
    colony_name: "大头一号",
    groups: [
      {
        date: "2026-09-15",
        checkins: [
          { checkin_id: 31, photos: [photo(11, "1/a11.jpg"), photo(12, "1/a12.jpg")] },
          { checkin_id: 30, photos: [photo(13, "1/a13.jpg")] },
        ],
      },
      {
        date: "2026-09-10",
        checkins: [{ checkin_id: 20, photos: [photo(14, "1/a14.jpg")] }],
      },
    ],
  },
  {
    colony_id: 2,
    colony_name: "针毛一号",
    groups: [
      { date: "2026-09-14", checkins: [{ checkin_id: 41, photos: [photo(21, "2/b21.jpg")] }] },
    ],
  },
];

const coloniesData: Colony[] = [
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
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
    checkin: { latest: null, baseline_date: null, days_since_last: null },
  },
];

type Mode = "desktop" | "web";

/** 挂载照片墙页（连同 ToastHost：错误轻提示走全局 store，宿主要在场才能断言）
 *  并等首次 photo_wall 拉取完成；mode 决定 Tauri 形态。 */
async function mountWall(data: PhotoWallColony[], mode: Mode = "desktop") {
  if (mode === "web") {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  } else {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  }
  if (loadPhotoBlobUrlMock.getMockImplementation() === undefined) {
    loadPhotoBlobUrlMock.mockImplementation(async (relPath: string) => `blob:u-${relPath}`);
  }
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "photo_wall":
        return data;
      case "list_colonies":
        return coloniesData;
      case "list_checkins":
        return [];
      case "get_photo_abs_dir":
        return "C:\\photos";
      default:
        return null;
    }
  });
  const w = mount({
    components: { PhotoWallPage, ToastHost },
    template: "<PhotoWallPage /><ToastHost />",
  });
  await flushPromises();
  return w;
}

beforeEach(() => {
  invokeMock.mockReset();
  loadPhotoBlobUrlMock.mockReset();
  revokeObjectUrlMock.mockReset();
  clearToasts();
  FakeIntersectionObserver.latest = null;
  (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("照片墙页 · 分区与排序（载荷契约）", () => {
  it("按窝分区、窝名标题、窝内日期分组与缩略图全序 = 载荷全序（不重排）", async () => {
    const w = await mountWall(wallData);
    fireIn(w);
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("photo_wall");
    const colonies = w.findAll(".wall-colony");
    expect(colonies.map((c) => c.find(".wall-colony-name").text())).toEqual(["大头一号", "针毛一号"]);

    // 窝内日期标题按载荷顺序（日期倒序由后端排好）
    const dates = colonies[0].findAll(".wall-day-date").map((d) => d.text());
    expect(dates).toEqual(["2026-09-15", "2026-09-10"]);

    // 缩略图全序 = 载荷全序：跨日期、跨登记、登记内按上传序
    const ids = w.findAll(".wall-thumb").map((t) => t.attributes("data-photo-id"));
    expect(ids).toEqual(["11", "12", "13", "14", "21"]);
  });

  it("全库无照片显示空态", async () => {
    const w = await mountWall([]);
    expect(w.find(".wall-empty").exists()).toBe(true);
    expect(w.find(".wall-empty").text()).toContain("还没有照片");
  });

  it("按窝筛选：选单窝只显示该窝，选回全部恢复", async () => {
    const w = await mountWall(wallData);
    fireIn(w);
    await flushPromises();

    // select.value 是字符串语义（DOM 规范），传串与真实浏览器行为一致；
    // 组件侧对 model 统一 Number 归一，双端（浏览器 option._value 原始值 /
    // happy-dom 属性串）都成立
    const select = w.find(".filter-select");
    await select.setValue("2");
    await flushPromises();
    expect(w.findAll(".wall-colony").map((c) => c.find(".wall-colony-name").text())).toEqual([
      "针毛一号",
    ]);
    expect(w.findAll(".wall-thumb").map((t) => t.attributes("data-photo-id"))).toEqual(["21"]);

    const selectBack = w.find(".filter-select");
    (selectBack.element as HTMLSelectElement).value = "";
    await selectBack.trigger("change");
    await flushPromises();
    expect(w.findAll(".wall-colony").length).toBe(2);
  });

  it("筛选后该窝无照片（数据刷新后筛选失效）给提示", async () => {
    const w = await mountWall([wallData[0]]);
    fireIn(w);
    await flushPromises();
    await w.find(".filter-select").setValue("1");

    // 数据刷新后 1 号窝没照片了（被删/照片全删），筛选仍停在 1 号窝
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "photo_wall" ? [wallData[1]] : null,
    );
    const pageVm = w.findComponent(PhotoWallPage).vm as { refresh: () => Promise<void> };
    await pageVm.refresh();
    await flushPromises();

    expect(w.find(".wall-empty").exists()).toBe(true);
    expect(w.find(".wall-empty").text()).toContain("该窝暂无照片");
  });
});

describe("照片墙页 · 懒加载与网页端取图", () => {
  it("未进视口不取图（网页端不调 loadPhotoBlobUrl，桌面不取照片根目录）", async () => {
    const w = await mountWall(wallData, "web");
    expect(loadPhotoBlobUrlMock).not.toHaveBeenCalled();
    // 视口外缩略图渲染占位容器，不渲染 img
    expect(w.find('.wall-thumb[data-photo-id="11"] img').exists()).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith("get_photo_abs_dir");
  });

  it("进视口才取图：只加载进视口的照片，加载完渲染 img", async () => {
    const w = await mountWall(wallData, "web");
    fireThumb(11);
    await flushPromises();

    expect(loadPhotoBlobUrlMock).toHaveBeenCalledTimes(1);
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledWith("1/a11.jpg");
    const img = w.find('.wall-thumb[data-photo-id="11"] img');
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("blob:u-1/a11.jpg");
    // 没进视口的仍是占位
    expect(w.find('.wall-thumb[data-photo-id="12"] img').exists()).toBe(false);
  });

  it("桌面模式进视口直接解析 asset 地址 + img loading=lazy", async () => {
    const w = await mountWall(wallData, "desktop");
    fireThumb(14);
    await flushPromises();

    expect(loadPhotoBlobUrlMock).not.toHaveBeenCalled();
    const img = w.find('.wall-thumb[data-photo-id="14"] img');
    expect(img.attributes("src")).toBe("asset://photos/1/a14.jpg");
    expect(img.attributes("loading")).toBe("lazy");
  });

  it("取图失败显示「文件缺失」占位，不崩溃（其余照片照常）", async () => {
    const w = await mountWall(wallData, "web");
    // mountWall 之后（挂载即拉载荷不会取图）、进视口之前装拒绝桩：
    // 仅 a12 拒绝（文件缺失），其余照常出图
    loadPhotoBlobUrlMock.mockImplementation(async (relPath: string) =>
      relPath === "1/a12.jpg" ? Promise.reject("HTTP 404 照片文件缺失") : `blob:u-${relPath}`,
    );
    fireIn(w);
    await flushPromises();

    const missing = w.find('.wall-thumb[data-photo-id="12"] .photo-missing');
    expect(missing.exists()).toBe(true);
    expect(missing.text()).toContain("文件缺失");
    // 其余照片照常渲染，页面没崩
    expect(w.find('.wall-thumb[data-photo-id="11"] img').exists()).toBe(true);
    expect(w.find('.wall-thumb[data-photo-id="21"] img').exists()).toBe(true);
  });

  it("滚出视口释放 blob（网页端），重新进视口再取", async () => {
    const w = await mountWall(wallData, "web");
    fireThumb(11);
    await flushPromises();
    expect(w.find('.wall-thumb[data-photo-id="11"] img').exists()).toBe(true);

    fireThumb(11, false);
    await flushPromises();
    expect(revokeObjectUrlMock).toHaveBeenCalledWith("blob:u-1/a11.jpg");
    expect(w.find('.wall-thumb[data-photo-id="11"] img').exists()).toBe(false);

    fireThumb(11);
    await flushPromises();
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledTimes(2);
    expect(w.find('.wall-thumb[data-photo-id="11"] img').exists()).toBe(true);
  });

  it("卸载释放全部已取的 blob", async () => {
    const w = await mountWall(wallData, "web");
    fireIn(w);
    await flushPromises();
    w.unmount();
    expect(revokeObjectUrlMock).toHaveBeenCalledTimes(5);
  });
});

describe("照片墙页 · 大图查看器", () => {
  it("点缩略图打开大图：显示窝名+日期，上一张/下一张按列表顺序连翻、越界禁用", async () => {
    const w = await mountWall(wallData, "desktop");
    fireIn(w);
    await flushPromises();

    await w.find('.wall-thumb[data-photo-id="11"] img').trigger("click");
    expect(w.find(".wall-viewer").exists()).toBe(true);
    expect(w.find(".viewer-caption").text()).toBe("大头一号 · 2026-09-15");
    expect((w.find(".viewer-prev").element as HTMLButtonElement).disabled).toBe(true);

    // 12（同日第二条登记，日期不变）→ 13 → 14（跨日期：09-15 → 09-10）→ 21（跨窝）
    await w.find(".viewer-next").trigger("click");
    expect(w.find(".viewer-img").attributes("alt")).toBe("p12.jpg");
    await w.find(".viewer-next").trigger("click");
    expect(w.find(".viewer-img").attributes("alt")).toBe("p13.jpg");
    expect(w.find(".viewer-caption").text()).toBe("大头一号 · 2026-09-15");
    await w.find(".viewer-next").trigger("click");
    expect(w.find(".viewer-caption").text()).toBe("大头一号 · 2026-09-10");
    await w.find(".viewer-next").trigger("click");
    expect(w.find(".viewer-caption").text()).toBe("针毛一号 · 2026-09-14");
    expect((w.find(".viewer-next").element as HTMLButtonElement).disabled).toBe(true);

    // 往回翻
    await w.find(".viewer-prev").trigger("click");
    expect(w.find(".viewer-img").attributes("alt")).toBe("p14.jpg");

    // 关闭
    await w.find(".viewer-close").trigger("click");
    expect(w.find(".wall-viewer").exists()).toBe(false);
  });

  it("连翻范围随当前筛选：筛单窝后大图只在 该窝 范围内翻", async () => {
    const w = await mountWall(wallData, "desktop");
    fireIn(w);
    await flushPromises();
    await w.find(".filter-select").setValue("2");
    await flushPromises();

    await w.find('.wall-thumb[data-photo-id="21"] img').trigger("click");
    expect(w.find(".viewer-caption").text()).toBe("针毛一号 · 2026-09-14");
    expect((w.find(".viewer-prev").element as HTMLButtonElement).disabled).toBe(true);
    expect((w.find(".viewer-next").element as HTMLButtonElement).disabled).toBe(true);
  });

  it("网页端大图连翻对未加载的照片按需取 blob", async () => {
    const w = await mountWall(wallData, "web");
    fireThumb(11);
    await flushPromises();

    await w.find('.wall-thumb[data-photo-id="11"] img').trigger("click");
    await w.find(".viewer-next").trigger("click");
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledWith("1/a12.jpg");
    await flushPromises();
    expect(w.find(".viewer-img").attributes("src")).toBe("blob:u-1/a12.jpg");
  });

  it("「查看登记」经 list_colonies 取窝对象后打开巢况时间线弹窗", async () => {
    const w = await mountWall(wallData, "desktop");
    fireIn(w);
    await flushPromises();

    await w.find('.wall-thumb[data-photo-id="11"] img').trigger("click");
    invokeMock.mockClear();
    await w.find(".viewer-checkin-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("list_colonies");
    const dlg = w.find(".checkin-dialog");
    expect(dlg.exists()).toBe(true);
    expect(dlg.find("h3").text()).toContain("大头一号");
    // 弹窗打开是天然反馈：大图查看器收起
    expect(w.find(".wall-viewer").exists()).toBe(false);
  });

  it("「查看登记」窝已被删除：失败轻提示兜底，不打开弹窗", async () => {
    const w = await mountWall(wallData, "desktop");
    fireIn(w);
    await flushPromises();

    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_colonies" ? [] : null,
    );
    await w.find('.wall-thumb[data-photo-id="11"] img').trigger("click");
    await w.find(".viewer-checkin-btn").trigger("click");
    await flushPromises();

    expect(w.find(".checkin-dialog").exists()).toBe(false);
    const toast = w.find(".toast-error");
    expect(toast.exists()).toBe(true);
    expect(toast.text()).toContain("登记");
  });

  it("页面只读：无上传/删除/裁剪任何写入口", async () => {
    const w = await mountWall(wallData, "desktop");
    fireIn(w);
    await flushPromises();

    expect(w.find(".photo-add-btn").exists()).toBe(false);
    expect(w.find(".photo-album-btn").exists()).toBe(false);
    expect(w.text()).not.toContain("传照片");
    expect(w.text()).not.toContain("裁剪");
    const cmds = new Set(invokeMock.mock.calls.map(([cmd]) => cmd));
    expect([...cmds].every((c) => ["photo_wall", "get_photo_abs_dir"].includes(c))).toBe(true);
  });
});
