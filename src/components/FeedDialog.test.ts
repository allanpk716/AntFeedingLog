/**
 * 票 04：FeedDialog 组件测试——喂食提交反馈按分类分支（spec F6 / 决策 D17）+
 * 食物多选列表易腐项记号。判定纯函数从组件同文件导出（涉及路径受限，不进 lib/）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import FeedDialog, { retrievalFeedback, retrievalFeedbackText } from "./FeedDialog.vue";
import DateTimeField from "./DateTimeField.vue";
import type { Colony, ColonyAction, FoodItem } from "../types";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

const colony: Colony = {
  id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20",
  status: "active", days_raised: 241, actions: [], recent: [], hibernation: null,
  checkin: { latest: null, baseline_date: null, days_since_last: null },
};
const action: ColonyAction = {
  action_id: 1, name: "喂食", icon: null, kind: "reminding", is_feeding: true,
  suggested_interval_days: null, days_since_last: 1, overdue: false, foods: [],
};

/** id=4 是脏数据（易腐但没配间隔）；id=5 停用（不该出现在多选列表） */
const foods: FoodItem[] = [
  { id: 1, name: "面包虫", enabled: true, sort: 1, suggested_interval_days: null, is_preset: true, referenced: true, perishable: true, retrieval_hours: 24 },
  { id: 2, name: "湿食", enabled: true, sort: 2, suggested_interval_days: null, is_preset: false, referenced: false, perishable: true, retrieval_hours: 6 },
  { id: 3, name: "种子", enabled: true, sort: 3, suggested_interval_days: 7, is_preset: true, referenced: true, perishable: false, retrieval_hours: null },
  { id: 4, name: "脏数据", enabled: true, sort: 4, suggested_interval_days: null, is_preset: false, referenced: false, perishable: true, retrieval_hours: null },
  { id: 5, name: "停用蜂蜜", enabled: false, sort: 5, suggested_interval_days: null, is_preset: false, referenced: false, perishable: true, retrieval_hours: 12 },
];

function mockInvoke() {
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === "list_foods" ? foods
      : cmd === "colony_month_records" ? []
        : cmd === "list_actions" ? []
          : cmd === "log_care" ? 7
            : null,
  );
}

/** 系统钟固定在 2026-09-19 10:00（本机时区）；只劫持 Date，不碰定时器（flushPromises 需真实微任务） */
function freezeClock() {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 8, 19, 10, 0));
}

async function mountDlg() {
  const w = mount(FeedDialog, { props: { colony, action } });
  await flushPromises(); // 挂载即拉 list_foods / 当月标记 / 操作名字表
  return w;
}

/** 点选名字含 name 的食物 chip */
async function pick(w: Awaited<ReturnType<typeof mountDlg>>, name: string) {
  const chip = (await w.findAll(".food")).find((b) => b.text().includes(name));
  expect(chip, `找不到食物 chip：${name}`).toBeDefined();
  await chip!.trigger("click");
}

/** 通过 DateTimeField 的 v-model 事件改补录时间 */
async function setTime(w: Awaited<ReturnType<typeof mountDlg>>, v: string) {
  await w.findComponent(DateTimeField).vm.$emit("update:modelValue", v);
}

beforeEach(() => {
  invokeMock.mockReset();
  freezeClock();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("撤食反馈判定（纯函数 retrievalFeedback）", () => {
  const now = "2026-09-19T10:00";

  it("所选不含易腐 → none（现状不变，无反馈句）", () => {
    const fb = retrievalFeedback([{ perishable: false, retrieval_hours: null }], "2026-09-19T10:00", now);
    expect(fb.kind).toBe("none");
    expect(retrievalFeedbackText(fb)).toBe("");
  });

  it("所选易腐全部有有效间隔且未逾期 → fresh，hours 取最短间隔（24 与 6 → 6）", () => {
    const fb = retrievalFeedback(
      [{ perishable: true, retrieval_hours: 24 }, { perishable: true, retrieval_hours: 6 }],
      "2026-09-19T10:00", now,
    );
    expect(fb).toEqual({ kind: "fresh", hours: 6 });
    expect(retrievalFeedbackText(fb)).toBe("将于 6 小时后提醒撤食");
  });

  it("发生时刻 + 最短间隔 恰好等于现在 → overdue（严格大于才算新鲜，不承诺将来时刻）", () => {
    const fb = retrievalFeedback([{ perishable: true, retrieval_hours: 24 }], "2026-09-18T10:00", now);
    expect(fb.kind).toBe("overdue");
    expect(retrievalFeedbackText(fb)).toBe("已逾期，明起每日提醒撤食");
  });

  it("补录 3 天前 → overdue", () => {
    const fb = retrievalFeedback([{ perishable: true, retrieval_hours: 24 }], "2026-09-16T10:00", now);
    expect(fb.kind).toBe("overdue");
  });

  it("含易腐但存在无有效间隔的（脏数据）→ none，不给将来时刻的承诺", () => {
    const mixed = [
      { perishable: true, retrieval_hours: 24 },
      { perishable: true, retrieval_hours: null },
    ];
    expect(retrievalFeedback(mixed, "2026-09-19T10:00", now).kind).toBe("none");
    // 非法域：0 / 越界 / 非整数（与设置守护 1–168 同口径）
    for (const bad of [0, -3, 169, 2.5]) {
      const fb = retrievalFeedback([{ perishable: true, retrieval_hours: bad }], "2026-09-19T10:00", now);
      expect(fb.kind, `retrieval_hours=${bad} 应视为无效`).toBe("none");
    }
  });

  it("发生时刻/现在坏值 → none 兜底", () => {
    expect(retrievalFeedback([{ perishable: true, retrieval_hours: 24 }], "乱写的", now).kind).toBe("none");
  });

  it("后端时刻格式（空格分隔含秒）也能比较", () => {
    // 发生 09-18 12:00 + 24h = 09-19 12:00，早于现在 13:00 → 已逾期
    const fb = retrievalFeedback([{ perishable: true, retrieval_hours: 24 }], "2026-09-18 12:00:00", "2026-09-19 13:00:00");
    expect(fb.kind).toBe("overdue");
  });
});

describe("FeedDialog 提交反馈分支", () => {
  it("选面包虫（24h）提交 → 反馈含「将于 24 小时后提醒撤食」，弹窗不自动关，点「知道了」才抛 saved", async () => {
    mockInvoke();
    const w = await mountDlg();
    await pick(w, "面包虫");
    await w.find(".record-btn").trigger("click");
    await flushPromises();

    const ok = w.find(".save-ok");
    expect(ok.exists()).toBe(true);
    expect(ok.text()).toContain("将于 24 小时后提醒撤食");
    expect(w.emitted("saved")).toBeUndefined(); // 留在弹窗里看反馈，不自动关

    await w.find(".record-btn").trigger("click"); // 按钮已换「知道了」
    await flushPromises();
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("面包虫+湿食（6h）→ 取最短：「将于 6 小时后提醒撤食」", async () => {
    mockInvoke();
    const w = await mountDlg();
    await pick(w, "面包虫");
    await pick(w, "湿食");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".save-ok").text()).toContain("将于 6 小时后提醒撤食");
  });

  it("补录 3 天前的面包虫 → 「已逾期，明起每日提醒撤食」（不给将来时刻承诺）", async () => {
    mockInvoke();
    const w = await mountDlg();
    await pick(w, "面包虫");
    await setTime(w, "2026-09-16T10:00");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".save-ok").text()).toContain("已逾期，明起每日提醒撤食");
    expect(w.find(".save-ok").text()).not.toContain("将于");
  });

  it("不选易腐（只选种子）→ 无撤食反馈句，提交后维持现状直接抛 saved", async () => {
    mockInvoke();
    const w = await mountDlg();
    await pick(w, "种子");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".save-ok").exists()).toBe(false);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("脏数据：易腐但无有效间隔 → 不显示撤食反馈句，直接抛 saved", async () => {
    mockInvoke();
    const w = await mountDlg();
    await pick(w, "脏数据");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".save-ok").exists()).toBe(false);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("提交失败（后端拒）不出成功反馈", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_foods" ? foods : cmd === "colony_month_records" ? [] : cmd === "list_actions" ? [] : Promise.reject("发生时间不能晚于当前时间"),
    );
    const w = await mountDlg();
    await pick(w, "面包虫");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").exists()).toBe(true);
    expect(w.find(".save-ok").exists()).toBe(false);
    expect(w.emitted("saved")).toBeUndefined();
  });
});

describe("食物多选列表易腐记号", () => {
  it("易腐项带圆点记号与悬停说明，非易腐没有；停用食物不进列表", async () => {
    mockInvoke();
    const w = await mountDlg();

    const chips = await w.findAll(".food");
    expect(chips).toHaveLength(4); // 停用蜂蜜不进新建入口

    const mealworm = chips.find((c) => c.text().includes("面包虫"))!;
    expect(mealworm.classes()).toContain("perishable");
    expect(mealworm.find(".p-dot").exists()).toBe(true);
    expect(mealworm.attributes("title")).toContain("24 小时");

    const seed = chips.find((c) => c.text().includes("种子"))!;
    expect(seed.classes()).not.toContain("perishable");
    expect(seed.find(".p-dot").exists()).toBe(false);
    expect(seed.attributes("title")).toBeUndefined();

    // 脏数据（无间隔）也有记号，悬停说明如实说明未设间隔
    const dirty = chips.find((c) => c.text().includes("脏数据"))!;
    expect(dirty.find(".p-dot").exists()).toBe(true);
    expect(dirty.attributes("title")).toContain("未设撤食间隔");
  });
});
