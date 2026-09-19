import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ColonyFormDialog from "./ColonyFormDialog.vue";
import type { Colony, ColonyAction, LocationItem } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 NestCheckinDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

/** 造一个操作 tile（真实 IPC 形状：启用操作才有 tile，Rust colony.rs「每个启用中操作一块」）。 */
function tile(overrides: Partial<ColonyAction> & { action_id: number; name: string }): ColonyAction {
  return {
    icon: null,
    kind: "log_only",
    is_feeding: false,
    suggested_interval_days: null,
    days_since_last: null,
    overdue: false,
    foods: [],
    ...overrides,
  };
}

const locations: LocationItem[] = [{ id: 1, name: "客厅", enabled: true, sort: 0 }];

function colony(actions: ColonyAction[]): Colony {
  return {
    id: 1,
    name: "大头一号",
    species: null,
    location_id: null,
    start_date: "2026-01-20",
    status: "active",
    days_raised: 241,
    actions,
    recent: [],
    hibernation: null,
    checkin: { latest: null, baseline_date: null, days_since_last: null },
  };
}

/** 该窝四个操作：喂食（启用）、加水（启用，已设每窝 7 天）、撤食（启用）、打扫（启用）。 */
const sampleActions = (): ColonyAction[] => [
  tile({ action_id: 11, name: "喂食", kind: "reminding", is_feeding: true }),
  tile({
    action_id: 12,
    name: "加水",
    interval_from_colony: true,
    effective_interval_days: 7,
  }),
  tile({ action_id: 13, name: "撤食", kind: "follow" }),
  tile({ action_id: 14, name: "打扫" }),
];

async function mountDlg(actions: ColonyAction[]) {
  invokeMock.mockImplementation(async () => null);
  const w = mount(ColonyFormDialog, {
    props: { editing: colony(actions), colonies: [colony(actions)], locations },
  });
  await flushPromises();
  return w;
}

function rowOf(w: ReturnType<typeof mount>, name: string) {
  const row = w.findAll(".interval-row").find((r) => r.text().includes(name));
  expect(row, `应有「${name}」一行`).toBeDefined();
  return row!;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("ColonyFormDialog「周期提醒」小节（每窝周期票 04）", () => {
  it("编辑模式：启用操作各一行，撤食(follow)不出现；已设行回显原始每窝周期", async () => {
    const w = await mountDlg(sampleActions());

    expect(w.text()).toContain("周期提醒");
    const names = w.findAll(".interval-row .interval-name").map((n) => n.text());
    expect(names).toEqual(["喂食", "加水", "打扫"]);
    expect(w.text()).not.toContain("撤食");

    const rows = w.findAll(".interval-row");
    expect((rows[0].find(".interval-input").element as HTMLInputElement).value).toBe("");
    expect((rows[1].find(".interval-input").element as HTMLInputElement).value).toBe("7");
    expect((rows[2].find(".interval-input").element as HTMLInputElement).value).toBe("");
  });

  it("新建模式（editing=null）：不出小节", async () => {
    invokeMock.mockImplementation(async () => null);
    const w = mount(ColonyFormDialog, { props: { editing: null, colonies: [], locations } });
    await flushPromises();

    expect(w.text()).not.toContain("周期提醒");
    expect(w.findAll(".interval-row")).toHaveLength(0);
  });

  it("校验：输入 400 点保存给人话报错，任何命令都不发（不落库）", async () => {
    const w = await mountDlg(sampleActions());

    await rowOf(w, "打扫").find(".interval-input").setValue("400");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("1–365");
    expect(w.emitted("saved")).toBeUndefined();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("设周期：未设行填 7 保存 → update_colony 后调 set_colony_action_interval(7)，抛 saved", async () => {
    const w = await mountDlg(sampleActions());

    await rowOf(w, "打扫").find(".interval-input").setValue("7");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("update_colony", expect.anything());
    expect(invokeMock).toHaveBeenCalledWith("set_colony_action_interval", {
      colonyId: 1,
      actionId: 14,
      intervalDays: 7,
    });
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("清周期：回显 7 的行点「清除」→ 输入框空，保存时 intervalDays 传 null", async () => {
    const w = await mountDlg(sampleActions());

    await rowOf(w, "加水").find(".interval-clear-btn").trigger("click");
    const input = rowOf(w, "加水").find(".interval-input");
    expect((input.element as HTMLInputElement).value).toBe("");

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_colony_action_interval", {
      colonyId: 1,
      actionId: 12,
      intervalDays: null,
    });
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("没动过的行不发命令：只改一行就只发一行", async () => {
    const w = await mountDlg(sampleActions());

    await rowOf(w, "打扫").find(".interval-input").setValue("30");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const intervalCalls = invokeMock.mock.calls.filter(
      ([cmd]) => cmd === "set_colony_action_interval",
    );
    expect(intervalCalls).toHaveLength(1);
    expect(intervalCalls[0][1]).toEqual({ colonyId: 1, actionId: 14, intervalDays: 30 });
  });

  it("无启用操作的窝：小节给出空态提示，保存不发周期命令", async () => {
    const w = await mountDlg([]);

    expect(w.text()).toContain("该窝暂无启用的操作");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "set_colony_action_interval"),
    ).toHaveLength(0);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("保存失败（后端拒绝，如库锁）：人话报错可见、不抛 saved", async () => {
    const w = await mountDlg(sampleActions());
    // mountDlg 会铺默认桩，这里在其后覆盖：周期命令拒绝（如库锁），其余照常
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "set_colony_action_interval") throw "库已锁";
      return null;
    });

    await rowOf(w, "打扫").find(".interval-input").setValue("7");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("库已锁");
    expect(w.emitted("saved")).toBeUndefined();
  });
});
