import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import NestCheckinDialog from "./NestCheckinDialog.vue";
import type { Colony, NestCheckin } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 QuickLogDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
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
