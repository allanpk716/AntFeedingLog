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
