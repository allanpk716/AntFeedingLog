import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import QuickLogDialog from "./QuickLogDialog.vue";
import type { Colony, ColonyAction } from "../types";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const colony: Colony = {
  id: 1, name: "大头一号", species: null, location_id: null, start_date: "2026-01-20",
  status: "active", days_raised: 241, actions: [], recent: [], hibernation: null,
};
const action: ColonyAction = {
  action_id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false,
  suggested_interval_days: null, days_since_last: 2, overdue: false, foods: [],
};

function mountDlg() {
  return mount(QuickLogDialog, { props: { colony, action } });
}

beforeEach(() => { invokeMock.mockReset(); });

describe("QuickLogDialog", () => {
  it("取消：直接关闭，不触发 log_care", async () => {
    const w = mountDlg();
    await w.find(".cancel-btn").trigger("click");
    expect(invokeMock).not.toHaveBeenCalled();
    expect(w.emitted("close")).toHaveLength(1);
  });

  it("提交：log_care 带默认现在时间、备注 null、空食物，成功后抛 saved", async () => {
    invokeMock.mockResolvedValue(3);
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
    invokeMock.mockResolvedValue(4);
    const w = mountDlg();
    await w.find(".time-input").setValue("2026-09-17T21:30");
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
