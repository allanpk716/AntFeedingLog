import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import QuickLogDialog from "./QuickLogDialog.vue";
import DateTimeField from "./DateTimeField.vue";
import { shiftMinutes } from "../lib/calendar";
import { todayIso } from "../lib/dates";
import type { Colony, ColonyAction } from "../types";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const colony: Colony = {
  id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20",
  status: "active", days_raised: 241, actions: [], recent: [], hibernation: null,
};
// 第二个操作（巢穴保湿）：灰点集成断言的名字来源——markers 名字表必须来自 colony.actions 全量
const hydrateAction: ColonyAction = {
  action_id: 3, name: "巢穴保湿", icon: null, kind: "log_only", is_feeding: false,
  suggested_interval_days: null, days_since_last: null, overdue: false, foods: [],
};
const action: ColonyAction = {
  action_id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false,
  suggested_interval_days: null, days_since_last: 2, overdue: false, foods: [],
};

function mountDlg() {
  return mount(QuickLogDialog, { props: { colony: { ...colony, actions: [action, hydrateAction] }, action } });
}

beforeEach(() => { invokeMock.mockReset(); });

describe("QuickLogDialog", () => {
  it("取消：直接关闭，不触发 log_care", async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "colony_month_records" ? [] : null));
    const w = mountDlg();
    await flushPromises(); // 挂载即拉当月标记（colony_month_records），等它落地再清调用
    invokeMock.mockClear();
    await w.find(".cancel-btn").trigger("click");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "log_care")).toBe(false);
    expect(w.emitted("close")).toHaveLength(1);
  });

  it("提交：log_care 带默认现在时间、备注 null、空食物，成功后抛 saved", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "colony_month_records" ? [] : cmd === "log_care" ? 3 : null,
    );
    const w = mountDlg();
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(call).toBeDefined();
    const input = call![1] as { input: Record<string, unknown> };
    expect(input.input.colony_id).toBe(1);
    expect(input.input.action_id).toBe(2);
    expect(input.input.happened_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
    expect(input.input.note).toBeNull();
    expect(input.input.food_ids).toEqual([]);
    expect(w.emitted("saved")).toHaveLength(1);
  });

  it("改补录时间 + 备注 trim 后提交", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "colony_month_records" ? [] : cmd === "log_care" ? 4 : null,
    );
    const w = mountDlg();
    await w.findComponent(DateTimeField).vm.$emit("update:modelValue", "2026-09-17T21:30");
    await w.find(".note-input").setValue("  顺手清了垃圾区  ");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const input = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care")![1] as { input: Record<string, unknown> };
    expect(input.input.happened_at).toBe("2026-09-17T21:30");
    expect(input.input.note).toBe("顺手清了垃圾区");
  });

  it("提交失败：错误展示在弹窗内、弹窗不关（未来时间被后端拒）", async () => {
    invokeMock.mockRejectedValue("发生时间不能晚于当前时间（2026-09-19T09:00 在未来）");
    const w = mountDlg();
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").text()).toContain("未来");
    expect(w.find(".quick-dialog").exists()).toBe(true);
    expect(w.emitted("saved")).toBeUndefined();
  });
});

describe("重复提醒（交互第三轮 #8）", () => {
  it("选中日已有同操作记录 → 黄条提醒，但仍可提交", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "colony_month_records") {
        // 用本地 todayIso()（自纠：toISOString 是 UTC，凌晨跑会差一天导致黄条不出现）
        return [{ day: +todayIso().slice(8, 10), action_id: 2, count: 1, last_time: `${todayIso()} 14:32:00` }];
      }
      if (cmd === "log_care") return 9;
      return null;
    });
    const w = mountDlg();
    await flushPromises();
    expect(w.find(".dup-warn").exists()).toBe(true);
    expect(w.find(".dup-warn").text()).toContain("已有 1 条");
    await w.find(".record-btn").trigger("click");
    await flushPromises();
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "log_care");
    expect(call).toBeDefined(); // 提醒不拦提交
  });

  it("无重复记录不出黄条", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "colony_month_records" ? [] : cmd === "log_care" ? 9 : null,
    );
    const w = mountDlg();
    await flushPromises();
    expect(w.find(".dup-warn").exists()).toBe(false);
  });

  it("标记：当天有其它操作 → 灰点渲染且 tip 带操作名（复审 #1/#2 集成断言）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "colony_month_records") {
        return [{ day: +todayIso().slice(8, 10), action_id: 3, count: 1, last_time: `${todayIso()} 09:00:00` }];
      }
      return null;
    });
    const w = mountDlg();
    await flushPromises();
    await w.find(".dp-trigger").trigger("click");
    const grey = w.find(".dp-day .dot:not(.cur)");
    expect(grey.exists()).toBe(true); // 名字表来自 colony.actions 全量 → 巢穴保湿不丢
    expect((grey.element.closest(".dp-day") as HTMLElement).title).toContain("巢穴保湿");
  });

  it("时间快捷键 −10分 生效（受控 v-model 断言 emit 值）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "colony_month_records" ? [] : null));
    const w = mountDlg();
    await flushPromises();
    const field = w.findComponent(DateTimeField);
    const before = field.props("modelValue") as string;
    await w.find(".dtf-m10").trigger("click");
    const emitted = field.emitted("update:modelValue");
    expect(emitted).toBeTruthy();
    // 计划原稿用 .at(-1)，项目 lib 是 ES2020（Array.prototype.at 是 ES2022）——改尾下标（同票 02 先例）
    const last = emitted![emitted!.length - 1] as unknown[];
    expect(last[0]).toBe(shiftMinutes(before, -10));
  });
});
