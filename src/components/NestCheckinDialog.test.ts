import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import NestCheckinDialog from "./NestCheckinDialog.vue";
import type { Colony, NestCheckin } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 QuickLogDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
// 票 08：网页端照片通路（blob 取图/HTTP 上传）打桩；photoSrc 等 桌面既有函数透传
const { uploadPhotosHttpMock, loadPhotoBlobUrlMock, revokeObjectUrlMock } = vi.hoisted(() => ({
  uploadPhotosHttpMock: vi.fn(),
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
    uploadPhotosHttp: uploadPhotosHttpMock,
    loadPhotoBlobUrl: loadPhotoBlobUrlMock,
    revokeObjectUrl: revokeObjectUrlMock,
  };
});

const colony: Colony = {
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
};

function checkin(overrides: Partial<NestCheckin> = {}): NestCheckin {
  return {
    id: 1,
    colony_id: 1,
    date: "2026-09-15",
    queen_count: 2,
    worker_count: 3000,
    moved_nest: false,
    note: "",
    created_at: "2026-09-15 21:00:00",
    photos: [],
    ...overrides,
  };
}

/** 挂载并等首次 list_checkins 拉取完成。 */
async function mountDlg(entries: NestCheckin[]) {
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_checkins") return entries;
    return null;
  });
  const w = mount(NestCheckinDialog, { props: { colony } });
  await flushPromises();
  return w;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("NestCheckinDialog（webui-checkin 票 02）", () => {
  it("挂载拉取 list_checkins，时间线按日期倒序渲染，最早一条标「基线」", async () => {
    const w = await mountDlg([
      checkin({ id: 3, date: "2026-09-17", queen_count: 2, note: "状态不错" }),
      checkin({ id: 2, date: "2026-09-15" }),
      checkin({ id: 1, date: "2026-09-01", worker_count: null, moved_nest: true }),
    ]);

    expect(invokeMock).toHaveBeenCalledWith("list_checkins", { colonyId: 1 });
    const entries = w.findAll(".entry");
    expect(entries.map((e) => e.attributes("data-checkin-id"))).toEqual(["3", "2", "1"]);
    // 基线 = 最早日期，标「基线」chip；其余不标
    const baselineChips = w.findAll(".baseline-chip");
    expect(baselineChips).toHaveLength(1);
    expect(entries[2].find(".baseline-chip").text()).toBe("基线");
    // 单条文案带数与备注
    expect(entries[0].text()).toContain("蚁后 2");
    expect(entries[0].text()).toContain("备注：状态不错");
    // 无照片本票不渲染照片区
    expect(entries[0].find(".entry-photos").exists()).toBe(false);
  });

  it("空时间线显示空态", async () => {
    const w = await mountDlg([]);
    expect(w.find(".checkin-empty").exists()).toBe(true);
    expect(w.find(".checkin-empty").text()).toContain("还没有巢况登记");
  });

  it("全空提交被前端拦截：不发 save_checkin，弹窗内提示", async () => {
    const w = await mountDlg([]);
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("至少填一项");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_checkin")).toBe(false);
  });

  it("只填备注可提交：save_checkin 带 snake_case 入参，成功后重拉时间线并抛 saved", async () => {
    const w = await mountDlg([]);
    await w.find(".date-input").setValue("2026-09-10"); // 补录过去日期
    await w.find(".note-input").setValue("  顺手数了蚁口  ");

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [checkin()];
      if (cmd === "save_checkin") return checkin();
      return null;
    });
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "save_checkin");
    expect(call).toBeDefined();
    expect(call![1]).toEqual({
      input: {
        colony_id: 1,
        date: "2026-09-10",
        queen_count: null,
        worker_count: null,
        moved_nest: false,
        note: "顺手数了蚁口",
      },
    });
    // 重拉时间线 + 通知外层刷新
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "list_checkins").length,
    ).toBeGreaterThanOrEqual(2);
    expect(w.emitted("saved")).toHaveLength(1);
    // 提交后表单回到新增态
    expect((w.find(".date-input").element as HTMLInputElement).value).not.toBe("2026-09-10");
  });

  it("蚁后数/工蚁数随表单提交；负数前端拦截不发 IPC", async () => {
    const w = await mountDlg([]);
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "list_checkins" ? [] : checkin()));

    await w.find(".queen-input").setValue("3");
    await w.find(".worker-input").setValue("1200");
    await w.find(".moved-input").setValue(true);
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "save_checkin");
    expect(call![1]).toEqual({
      input: {
        colony_id: 1,
        date: expect.any(String),
        queen_count: 3,
        worker_count: 1200,
        moved_nest: true,
        note: null,
      },
    });

    invokeMock.mockClear();
    await w.find(".queen-input").setValue("-1");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").text()).toContain("非负整数");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_checkin")).toBe(false);
  });

  it("编辑：预填当前值，提交走 update_checkin 带 id，随后回新增态", async () => {
    const w = await mountDlg([
      checkin({ id: 5, date: "2026-09-15", queen_count: 2, worker_count: 3000 }),
    ]);
    await w.find(".entry-edit-btn").trigger("click");

    expect((w.find(".date-input").element as HTMLInputElement).value).toBe("2026-09-15");
    expect((w.find(".queen-input").element as HTMLInputElement).value).toBe("2");
    expect((w.find(".worker-input").element as HTMLInputElement).value).toBe("3000");
    expect((w.find(".moved-input").element as HTMLInputElement).checked).toBe(false);

    await w.find(".date-input").setValue("2026-09-12"); // 首条日期可改
    await w.find(".worker-input").setValue("");
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [checkin({ date: "2026-09-12" })];
      if (cmd === "update_checkin") return checkin({ date: "2026-09-12", worker_count: null });
      return null;
    });
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "update_checkin");
    expect(call![1]).toEqual({
      id: 5,
      input: {
        date: "2026-09-12",
        queen_count: 2,
        worker_count: null,
        moved_nest: false,
        note: null,
      },
    });
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_checkin")).toBe(false);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("取消编辑回新增态：表单清空、提交走 save_checkin", async () => {
    const w = await mountDlg([checkin({ id: 5 })]);
    await w.find(".entry-edit-btn").trigger("click");
    await w.find(".cancel-edit-btn").trigger("click");

    expect((w.find(".queen-input").element as HTMLInputElement).value).toBe("");
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "list_checkins" ? [] : checkin()));
    await w.find(".note-input").setValue("新登记");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_checkin")).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "update_checkin")).toBe(false);
  });

  it("删除两段确认：第一次只进入确认态，第二次才发 delete_checkin", async () => {
    const w = await mountDlg([checkin({ id: 7 })]);
    const delBtn = w.find(".entry-delete-btn");

    invokeMock.mockClear();
    await delBtn.trigger("click");
    expect(w.find(".entry-delete-btn").text()).toBe("确认删除？");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "delete_checkin")).toBe(false);

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [];
      return null;
    });
    await w.find(".entry-delete-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("delete_checkin", { id: 7 });
    expect(w.emitted("saved")).toHaveLength(1);
    expect(w.findAll(".entry")).toHaveLength(0);
  });

  it("提交失败：错误展示在弹窗内、表单值保留、弹窗不关", async () => {
    const w = await mountDlg([]);
    invokeMock.mockRejectedValue("登记日期不能晚于今天（2026-09-19 在未来）");
    await w.find(".date-input").setValue("2026-09-19");
    await w.find(".note-input").setValue("提前写好");
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("未来");
    expect((w.find(".note-input").element as HTMLTextAreaElement).value).toBe("提前写好");
    expect(w.emitted("saved")).toBeUndefined();
    expect(w.find(".checkin-dialog").exists()).toBe(true);
  });

  it("点遮罩关闭不落库", async () => {
    const w = await mountDlg([]);
    invokeMock.mockClear();
    await w.find(".overlay").trigger("click");
    expect(w.emitted("close")).toHaveLength(1);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

// ── 巢况照片（webui-checkin 票 07）：网格 / 缺图占位 / 上传 / 删除连带文案 ──

describe("NestCheckinDialog 巢况照片（webui-checkin 票 07）", () => {
  const PHOTO_DIR = "C:\\data\\photos";
  const REL = "1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg";
  const photoMeta = {
    id: 11,
    checkin_id: 7,
    rel_path: REL,
    original_name: "IMG_001.jpg",
    note: "",
  };

  /** 桌面 WebView 形态：注入 __TAURI_INTERNALS__（isTauri 判定 + convertFileSrc）。 */
  function stubTauriInternals() {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
      convertFileSrc: (p: string) => `http://asset.localhost/${encodeURIComponent(p)}`,
    };
  }

  async function mountWithPhotos(
    entries: NestCheckin[],
    photoDir: string | null = PHOTO_DIR,
  ) {
    stubTauriInternals();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return entries;
      if (cmd === "get_photo_abs_dir") return photoDir;
      return null;
    });
    const w = mount(NestCheckinDialog, { props: { colony } });
    await flushPromises();
    return w;
  }

  it("有照片的登记渲染缩略图网格（asset 协议 URL = 照片根目录 + 相对路径），无照片不渲染", async () => {
    const w = await mountWithPhotos([
      checkin({ id: 7, photos: [photoMeta] }),
      checkin({ id: 8, date: "2026-09-10", photos: [] }),
    ]);

    expect(invokeMock).toHaveBeenCalledWith("get_photo_abs_dir");
    const grids = w.findAll(".entry-photos");
    expect(grids).toHaveLength(1); // 无照片的登记不渲染照片区
    const img = grids[0].find("img.photo-thumb");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe(
      `http://asset.localhost/${encodeURIComponent("C:\\data\\photos/1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg")}`,
    );
  });

  it("照片根目录取不到 → 「预览不可用」占位，不崩溃", async () => {
    const w = await mountWithPhotos([checkin({ id: 7, photos: [photoMeta] })], null);
    expect(w.find(".photo-unavailable").exists()).toBe(true);
    expect(w.find(".photo-unavailable").text()).toContain("预览不可用");
  });

  it("图片加载失败（文件缺失）→ 占位符提示，不崩溃", async () => {
    const w = await mountWithPhotos([checkin({ id: 7, photos: [photoMeta] })]);
    expect(w.find("img.photo-thumb").exists()).toBe(true);

    await w.find("img.photo-thumb").trigger("error");
    await flushPromises();

    const missing = w.find(".photo-missing");
    expect(missing.exists()).toBe(true);
    expect(missing.text()).toContain("文件缺失");
    expect(w.find("img.photo-thumb").exists()).toBe(false);
  });

  it("传照片：pick_photo_files → attach_photos(checkinId, paths) → 重拉时间线并抛 saved", async () => {
    const w = await mountWithPhotos([checkin({ id: 7 })]);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [checkin({ id: 7, photos: [photoMeta] })];
      if (cmd === "pick_photo_files") return ["C:/pics/a.jpg", "C:/pics/b.png"];
      if (cmd === "get_photo_abs_dir") return PHOTO_DIR;
      if (cmd === "attach_photos") return [photoMeta];
      return null;
    });

    await w.find(".photo-add-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("pick_photo_files");
    expect(invokeMock).toHaveBeenCalledWith("attach_photos", {
      checkinId: 7,
      paths: ["C:/pics/a.jpg", "C:/pics/b.png"],
    });
    // 成功后重拉时间线
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "list_checkins").length,
    ).toBeGreaterThanOrEqual(2);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("取消选择（pick_photo_files 返回 null）不发 attach_photos", async () => {
    const w = await mountWithPhotos([checkin({ id: 7 })]);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [checkin({ id: 7 })];
      if (cmd === "pick_photo_files") return null;
      if (cmd === "get_photo_abs_dir") return PHOTO_DIR;
      return null;
    });

    await w.find(".photo-add-btn").trigger("click");
    await flushPromises();

    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "attach_photos")).toBe(false);
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("上传失败：错误展示在弹窗内，不抛 saved", async () => {
    const w = await mountWithPhotos([checkin({ id: 7 })]);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [checkin({ id: 7 })];
      if (cmd === "pick_photo_files") return ["C:/pics/a.jpg"];
      if (cmd === "get_photo_abs_dir") return PHOTO_DIR;
      if (cmd === "attach_photos") throw "a.jpg: 仅支持 JPEG / PNG / WebP 图片";
      return null;
    });

    await w.find(".photo-add-btn").trigger("click");
    await flushPromises();

    expect(w.find(".photo-error").text()).toContain("JPEG / PNG / WebP");
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("删除带照片的登记：两段确认文案补「该登记的 N 张照片将一并删除」", async () => {
    const w = await mountWithPhotos([
      checkin({ id: 7, photos: [photoMeta, { ...photoMeta, id: 12, rel_path: "1/other.jpg" }] }),
      checkin({ id: 8, date: "2026-09-10" }),
    ]);

    // 无照片的登记不出提示
    const entries = w.findAll(".entry");
    await entries[1].find(".entry-delete-btn").trigger("click");
    expect(entries[1].find(".delete-photo-warn").exists()).toBe(false);

    // 带照片的登记：第一次进入确认态即出提示，第二次才真删
    await entries[0].find(".entry-delete-btn").trigger("click");
    expect(entries[0].find(".delete-photo-warn").text()).toContain("该登记的 2 张照片将一并删除");
    expect(entries[0].find(".entry-delete-btn").text()).toBe("确认删除？");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "delete_checkin")).toBe(false);

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_checkins") return [];
      return null;
    });
    await entries[0].find(".entry-delete-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("delete_checkin", { id: 7 });
  });

  it("点缩略图开大图查看器，点遮罩关闭", async () => {
    const w = await mountWithPhotos([checkin({ id: 7, photos: [photoMeta] })]);
    expect(w.find(".photo-viewer").exists()).toBe(false);

    await w.find("img.photo-thumb").trigger("click");
    expect(w.find(".photo-viewer").exists()).toBe(true);
    expect(w.find(".photo-viewer-name").text()).toContain("IMG_001.jpg");

    await w.find(".photo-viewer").trigger("click");
    expect(w.find(".photo-viewer").exists()).toBe(false);
  });
});

// ── 网页端（webui-checkin 票 08）：浏览器照片通路 + 手机竖屏断点类 ─────────

describe("NestCheckinDialog 网页端（webui-checkin 票 08）", () => {
  const REL = "1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg";
  const browserPhoto = {
    id: 11,
    checkin_id: 7,
    rel_path: REL,
    original_name: "ok.jpg",
    note: "",
  };

  beforeEach(() => {
    invokeMock.mockReset();
    uploadPhotosHttpMock.mockReset();
    loadPhotoBlobUrlMock.mockReset();
    revokeObjectUrlMock.mockReset();
    // 浏览器形态：不注入 __TAURI_INTERNALS__（isTauri() === false）
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  /** 浏览器形态挂载：list_checkins 回 entries，其余命令回 null。 */
  async function mountBrowser(entries: NestCheckin[]) {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_checkins" ? entries : null,
    );
    const w = mount(NestCheckinDialog, { props: { colony } });
    await flushPromises();
    return w;
  }

  it("竖屏断点类挂载（≤480px 媒体查询的落点，断点类名可测）", async () => {
    const w = await mountBrowser([]);
    expect(w.find(".vp-form-stack").exists()).toBe(true);
  });

  it("上传入口是 file input：accept=image/* + multiple + capture=environment（手机直调相机）；点按钮不调 pick_photo_files", async () => {
    const w = await mountBrowser([checkin({ id: 7 })]);
    const input = w.find(".photo-file-input");
    expect(input.exists()).toBe(true);
    expect(input.attributes("accept")).toBe("image/*");
    expect(input.attributes("multiple")).toBeDefined();
    expect(input.attributes("capture")).toBe("environment");

    invokeMock.mockClear();
    await w.find(".photo-add-btn").trigger("click");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "pick_photo_files")).toBe(false);
  });

  it("手机双入口（终局评审）：「拍照」input 带 capture=environment，「从相册选」input 不带 capture", async () => {
    const w = await mountBrowser([checkin({ id: 7 })]);

    // 拍照入口：capture=environment 直调相机
    const camera = w.find(".photo-file-input");
    expect(camera.attributes("capture")).toBe("environment");
    expect(camera.attributes("accept")).toBe("image/*");
    expect(camera.attributes("multiple")).toBeDefined();

    // 从相册选入口：不带 capture（系统弹相册/文件选择），其余属性同款
    const album = w.find(".photo-album-input");
    expect(album.exists()).toBe(true);
    expect(album.attributes("accept")).toBe("image/*");
    expect(album.attributes("multiple")).toBeDefined();
    expect(album.attributes("capture")).toBeUndefined();
  });

  it("「从相册选」点按钮不调 pick_photo_files，选完文件同样走 uploadPhotosHttp（与拍照共用处理函数）", async () => {
    const w = await mountBrowser([checkin({ id: 7 })]);
    uploadPhotosHttpMock.mockResolvedValue([{ ...browserPhoto }]);
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_checkins" ? [checkin({ id: 7, photos: [{ ...browserPhoto }] })] : null,
    );

    invokeMock.mockClear();
    await w.find(".photo-album-btn").trigger("click");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "pick_photo_files")).toBe(false);

    const files = [new File(["c"], "c.jpg")];
    const input = w.find(".photo-album-input");
    Object.defineProperty(input.element, "files", { value: files, configurable: true });
    await input.trigger("change");
    await flushPromises();

    expect(uploadPhotosHttpMock).toHaveBeenCalledWith(7, files);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("选文件后 uploadPhotosHttp(checkinId, files) → 重拉时间线 + 抛 saved；input 值清空可重选同一批", async () => {
    const w = await mountBrowser([checkin({ id: 7 })]);
    uploadPhotosHttpMock.mockResolvedValue([{ ...browserPhoto }]);
    // 上传成功后的重拉：时间线带上新照片
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_checkins" ? [checkin({ id: 7, photos: [{ ...browserPhoto }] })] : null,
    );

    // 先点「传照片」记下目标登记（真实交互顺序），再模拟选完文件触发 change
    await w.find(".photo-add-btn").trigger("click");
    const files = [new File(["a"], "a.jpg"), new File(["b"], "b.png")];
    const input = w.find(".photo-file-input");
    Object.defineProperty(input.element, "files", { value: files, configurable: true });
    await input.trigger("change");
    await flushPromises();

    expect(uploadPhotosHttpMock).toHaveBeenCalledWith(7, files);
    expect(w.emitted("saved")).toHaveLength(1);
    expect((input.element as HTMLInputElement).value).toBe("");
    // 时间线重拉出照片缩略图（blob 通路在下一用例细验）
    expect(w.findAll(".entry-photos").length).toBe(1);
  });

  it("上传失败（批量可能部分成功）：错误展示 + 仍重拉时间线，不抛 saved", async () => {
    const w = await mountBrowser([checkin({ id: 7 })]);
    uploadPhotosHttpMock.mockRejectedValue("一次最多上传 9 张照片");

    await w.find(".photo-add-btn").trigger("click");
    const input = w.find(".photo-file-input");
    Object.defineProperty(input.element, "files", {
      value: [new File(["a"], "a.jpg")],
      configurable: true,
    });
    await input.trigger("change");
    await flushPromises();

    expect(w.find(".photo-error").text()).toContain("最多上传 9 张");
    // 失败分支也重拉（初始挂载一次 + 失败后一次）：已落库的部分照片立即出现
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "list_checkins").length,
    ).toBeGreaterThanOrEqual(2);
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("浏览器缩略图走 loadPhotoBlobUrl 的 objectURL；取图失败标「文件缺失」占位", async () => {
    const okRel = "1/6ba7b810-9dad-11d1-80b4-00c04fd430c8.jpg";
    loadPhotoBlobUrlMock.mockImplementation(async (rel: string) =>
      rel === okRel ? "blob:ok-url" : Promise.reject("HTTP 404"),
    );
    const w = await mountBrowser([
      checkin({
        id: 7,
        photos: [
          { ...browserPhoto, rel_path: okRel },
          { ...browserPhoto, id: 12, rel_path: "1/gone.jpg", original_name: null },
        ],
      }),
    ]);
    await flushPromises();

    expect(loadPhotoBlobUrlMock).toHaveBeenCalledWith(okRel);
    expect(loadPhotoBlobUrlMock).toHaveBeenCalledWith("1/gone.jpg");
    expect(w.find('img[title="ok.jpg"]').attributes("src")).toBe("blob:ok-url");
    expect(w.find(".photo-missing").exists()).toBe(true);
  });

  it("瞬时取图失败后重取成功：占位恢复为图片（终局评审：missingPhotoIds 取图成功即移除，不滞留）", async () => {
    // 第一次取图失败（瞬时机 网络/服务抖动）→ 标「文件缺失」占位
    loadPhotoBlobUrlMock.mockRejectedValueOnce("HTTP 500");
    loadPhotoBlobUrlMock.mockResolvedValue("blob:retry-ok");
    const w = await mountBrowser([checkin({ id: 7, photos: [{ ...browserPhoto }] })]);
    await flushPromises();
    expect(w.find(".photo-missing").exists()).toBe(true);
    expect(w.find("img.photo-thumb").exists()).toBe(false);

    // 任一重拉路径（这里补一条登记触发 load）→ 同一张照片重取成功 → 占位恢复为图片
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_checkins" ? [checkin({ id: 7, photos: [{ ...browserPhoto }] })] : null,
    );
    await w.find(".note-input").setValue("再登记一次触发重拉");
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    expect(loadPhotoBlobUrlMock).toHaveBeenCalledTimes(2);
    expect(w.find('img[title="ok.jpg"]').attributes("src")).toBe("blob:retry-ok");
    expect(w.find(".photo-missing").exists()).toBe(false);
  });

  it("组件卸载释放全部 objectURL（不泄漏）", async () => {
    loadPhotoBlobUrlMock.mockResolvedValue("blob:bye");
    const w = await mountBrowser([checkin({ id: 7, photos: [{ ...browserPhoto }] })]);
    await flushPromises();
    expect(w.find('img[title="ok.jpg"]').exists()).toBe(true);

    w.unmount();
    expect(revokeObjectUrlMock).toHaveBeenCalledWith("blob:bye");
  });
});
