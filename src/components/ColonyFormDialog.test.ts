import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ColonyFormDialog from "./ColonyFormDialog.vue";
import type { CareActionItem, Colony, ColonyAction, LocationItem } from "../types";

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

function colony(actions: ColonyAction[], hydration_method: Colony["hydration_method"] = null): Colony {
  return {
    id: 1,
    name: "大头一号",
    species: null,
    location_id: null,
    start_date: "2026-01-20",
    status: "active",
    days_raised: 241,
    hydration_method,
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

// ── 保湿方式（保湿方式票 02，规格状态矩阵四态九规则）─────────────────────

/** 该窝三个操作：喂食、巢穴保湿（锚行）、打扫。 */
const hydrationActions = (): ColonyAction[] => [
  tile({ action_id: 11, name: "喂食", kind: "reminding", is_feeding: true }),
  tile({ action_id: 15, name: "巢穴保湿" }),
  tile({ action_id: 14, name: "打扫" }),
];

/** 操作字典里的巢穴保湿项（list_actions 形状）。 */
const hydrationDictItem: CareActionItem = {
  id: 15,
  name: "巢穴保湿",
  icon: null,
  kind: "log_only",
  is_feeding: false,
  suggested_interval_days: null,
  enabled: true,
  sort: 5,
  is_preset: true,
  referenced: false,
};

async function mountEdit(
  hydration_method: Colony["hydration_method"],
  intervalDays: number | null,
) {
  invokeMock.mockImplementation(async () => null);
  const actions = hydrationActions();
  if (intervalDays !== null) {
    actions[1] = {
      ...actions[1],
      interval_from_colony: true,
      effective_interval_days: intervalDays,
    };
  }
  const w = mount(ColonyFormDialog, {
    props: { editing: colony(actions, hydration_method), colonies: [colony(actions, hydration_method)], locations },
  });
  await flushPromises();
  return w;
}

async function mountCreate(dict: unknown[] = [hydrationDictItem]) {
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === "list_actions" ? dict : null,
  );
  const w = mount(ColonyFormDialog, {
    props: { editing: null, colonies: [], locations },
  });
  await flushPromises();
  return w;
}

function methodSelect(w: ReturnType<typeof mount>) {
  return w.find(".hydration-method-select");
}

/** 选中项的原始绑定值（Vue 存在 option._value 上；null=未设，与 v-model 同语义）。 */
function pickedMethod(w: ReturnType<typeof mount>): unknown {
  const sel = methodSelect(w).element as HTMLSelectElement;
  const opt = sel.selectedOptions[0] as (HTMLOptionElement & { _value?: unknown }) | undefined;
  expect(opt, "方式下拉应有选中项").toBeDefined();
  return "_value" in opt! ? opt!._value : sel.value;
}

/** 切方式：value 传 "manual"/"tower"/"未设"（null 选项无 value 属性，DOM 值回落为文本）。 */
async function pickMethod(w: ReturnType<typeof mount>, value: string) {
  await methodSelect(w).setValue(value);
  await flushPromises(); // 方式变化经 watch 重算天数框
}

function hydrationInput(w: ReturnType<typeof mount>) {
  return w.find("input[aria-label='巢穴保湿的每窝周期（天）']");
}

function colonyInputPayload(_w: ReturnType<typeof mount>, cmd: string) {
  const call = invokeMock.mock.calls.find(([c]) => c === cmd);
  expect(call, `应发出 ${cmd}`).toBeDefined();
  return (call![1] as { input: Record<string, unknown> }).input;
}

describe("保湿方式：编辑回显（规则 1，四态原样、打开不自动预填）", () => {
  it("A 态（未设+无周期）：方式未设、天数框空", async () => {
    const w = await mountEdit(null, null);
    expect(methodSelect(w).element as HTMLSelectElement).toBeDefined();
    expect(pickedMethod(w)).toBeNull();
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("");
  });

  it("B 态（未设+已有周期 5）：方式未设、天数照常显示 5", async () => {
    const w = await mountEdit(null, 5);
    expect(pickedMethod(w)).toBeNull();
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("5");
  });

  it("C 态（手动加水+7）：方式显示手动加水、天数 7", async () => {
    const w = await mountEdit("manual", 7);
    expect(pickedMethod(w)).toBe("manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");
  });

  it("D 态（水塔+空）：方式显示水塔、天数框空", async () => {
    const w = await mountEdit("tower", null);
    expect(pickedMethod(w)).toBe("tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("");
  });
});

describe("保湿方式：编辑交互（规则 2/3/4/6 + F4/F5）", () => {
  it("规则 2：B 态选方式不覆盖库内周期，保存只换方式不带周期行", async () => {
    const w = await mountEdit(null, 5);

    await pickMethod(w, "manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("5");

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "update_colony");
    expect(input.hydration_method).toBe("manual");
    expect(input.interval_changes).toEqual([]);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("规则 2：A 态空框选方式 → 预填默认；保存随 update_colony 一次落库", async () => {
    const w = await mountEdit(null, null);

    await pickMethod(w, "tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("15");

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "update_colony");
    expect(input.hydration_method).toBe("tower");
    expect(input.interval_changes).toEqual([{ action_id: 15, interval_days: 15 }]);
  });

  it("规则 3：会话预填值未手工改 → 切换方式跟随新默认", async () => {
    const w = await mountEdit(null, null);

    await pickMethod(w, "manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");
    await pickMethod(w, "tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("15");
  });

  it("规则 3：手工改过的值 → 切换方式不动", async () => {
    const w = await mountEdit(null, null);

    await pickMethod(w, "manual");
    await hydrationInput(w).setValue("9");
    await pickMethod(w, "tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("9");
  });

  it("F5：D 态空框直接切换方式 → 空框即预填新方式默认", async () => {
    const w = await mountEdit("tower", null);

    await pickMethod(w, "manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");
  });

  it("F4 净零：库内 B 态（未设+周期）选了又改回未设 → 框不动、保存不带周期行（B 仍 B）", async () => {
    const w = await mountEdit(null, 5);

    await pickMethod(w, "manual");
    await pickMethod(w, "未设");
    expect(pickedMethod(w)).toBeNull();
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("5");

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "update_colony");
    expect(input.hydration_method).toBeNull();
    expect(input.interval_changes).toEqual([]);
  });

  it("规则 4：库内 C 态清回未设 → 框同步清空置灰；保存方式置未设、周期删行走后端方式路径", async () => {
    const w = await mountEdit("manual", 7);

    await pickMethod(w, "未设");
    const input = hydrationInput(w);
    expect((input.element as HTMLInputElement).value).toBe("");
    expect((input.element as HTMLInputElement).disabled).toBe(true);

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const payload = colonyInputPayload(w, "update_colony");
    expect(payload.hydration_method).toBeNull();
    // 删周期由后端 clear_hydration 同事务执行，前端不重复提交清除
    expect(payload.interval_changes).toEqual([]);
  });

  it("规则 6：只清天数 → 方式保留，保存经 interval_changes 传 null 删行（C→D）", async () => {
    const w = await mountEdit("manual", 7);

    await rowOf(w, "巢穴保湿").find(".interval-clear-btn").trigger("click");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("");

    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const payload = colonyInputPayload(w, "update_colony");
    expect(payload.hydration_method).toBe("manual");
    expect(payload.interval_changes).toEqual([{ action_id: 15, interval_days: null }]);
  });

  it("整组校验：保湿行天数非法 → 人话报错带操作名，任何命令都不发", async () => {
    const w = await mountEdit("manual", 7);

    await hydrationInput(w).setValue("400");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("巢穴保湿");
    expect(w.find(".form-error").text()).toContain("1–365");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("说明文案：选择器旁有「选方式→预填→可改」的提示", async () => {
    const w = await mountEdit(null, null);
    expect(w.text()).toContain("自动填建议周期");
  });
});

describe("保湿方式：新建（D3 随建随落，create_colony 原子）", () => {
  it("未选方式：不出天数框、保存不落周期", async () => {
    const w = await mountCreate();

    expect(w.text()).toContain("保湿方式");
    expect(hydrationInput(w).exists()).toBe(false);

    await w.find("input.name-input").setValue("新窝");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "create_colony");
    expect(input.hydration_method).toBeNull();
    expect(input.interval_changes).toEqual([]);
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "set_colony_action_interval"),
    ).toHaveLength(0);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("选方式：天数框出现并预填默认、可改；保存随 create_colony 一次落库（不拆两步命令）", async () => {
    const w = await mountCreate();

    await pickMethod(w, "manual");
    expect(hydrationInput(w).exists()).toBe(true);
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");

    await hydrationInput(w).setValue("9");
    await w.find("input.name-input").setValue("新窝");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "create_colony");
    expect(input.hydration_method).toBe("manual");
    expect(input.interval_changes).toEqual([{ action_id: 15, interval_days: 9 }]);
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "set_colony_action_interval"),
    ).toHaveLength(0);
  });

  it("切换方式按规则 3 跟随会话预填值", async () => {
    const w = await mountCreate();

    await pickMethod(w, "manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");
    await pickMethod(w, "tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("15");
  });

  it("规则 3：手工改过的天数 → 切换方式不覆盖（新建同款），保存载荷带手工值", async () => {
    const w = await mountCreate();

    await pickMethod(w, "manual");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("7");
    await hydrationInput(w).setValue("9");
    await pickMethod(w, "tower");
    expect((hydrationInput(w).element as HTMLInputElement).value).toBe("9");

    await w.find("input.name-input").setValue("新窝");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "create_colony");
    expect(input.hydration_method).toBe("tower");
    expect(input.interval_changes).toEqual([{ action_id: 15, interval_days: 9 }]);
  });

  it("保湿天数非法同样被整组校验拦下（不落库）", async () => {
    const w = await mountCreate();

    await pickMethod(w, "manual");
    await hydrationInput(w).setValue("400");
    await w.find("input.name-input").setValue("新窝");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    expect(w.find(".form-error").text()).toContain("巢穴保湿");
    // 挂载时的 list_actions 之外，任何写命令都不发（不落库）
    expect(
      invokeMock.mock.calls.filter(([cmd]) =>
        ["create_colony", "update_colony", "set_colony_action_interval"].includes(cmd as string),
      ),
    ).toHaveLength(0);
  });

  it("字典里锚不到巢穴保湿（改名/停用）：方式仍可选，只设标签不落初始周期", async () => {
    const w = await mountCreate([]);

    await pickMethod(w, "tower");
    expect(hydrationInput(w).exists()).toBe(false);

    await w.find("input.name-input").setValue("新窝");
    await w.find(".submit-btn").trigger("click");
    await flushPromises();

    const input = colonyInputPayload(w, "create_colony");
    expect(input.hydration_method).toBe("tower");
    expect(input.interval_changes).toEqual([]);
  });
});
