import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ColonyCard from "./ColonyCard.vue";
import type { Colony, ColonyAction } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 QuickLogDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
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

function mountCard() {
  return mount(ColonyCard, { props: { colony } });
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
