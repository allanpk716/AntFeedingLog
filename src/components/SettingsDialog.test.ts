import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import SettingsDialog from "./SettingsDialog.vue";

// 不依赖 Tauri 运行时：mock 掉 IPC 与事件监听（沿 LogListPage.test.ts 先例）
const { invokeMock, listenStub } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenStub: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenStub }));

function baseMock(updateState: object = { status: "idle" }) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_actions":
      case "list_foods":
      case "list_locations":
        return [];
      case "get_settings":
        return {
          notify_master_enabled: true,
          notify_overdue_enabled: true,
          notify_hibernation_enabled: true,
          wake_remind_days_ahead: 7,
          autostart_enabled: true,
        };
      case "get_app_version":
        return "0.1.0";
      case "get_update_state":
        return updateState;
      default:
        return null;
    }
  });
}

async function openUpdateTab(updateState?: object) {
  baseMock(updateState);
  const wrapper = mount(SettingsDialog);
  await flushPromises();
  await wrapper.find(".tab-update").trigger("click");
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  listenStub.mockClear();
});

describe("设置弹窗「更新」节（票 06）", () => {
  it("新增「更新」tab：点开显示当前版本与检查入口（验收 1 的挂载面）", async () => {
    const wrapper = await openUpdateTab();

    expect(wrapper.find(".tab-update").exists()).toBe(true);
    expect(wrapper.find(".current-version").text()).toContain("v0.1.0");
    expect(wrapper.find(".check-btn").exists()).toBe(true);
  });

  it("update_state 为失败残留 → 本节顶部出「上次升级未完成」引导文案（验收 3）", async () => {
    const wrapper = await openUpdateTab({ status: "last_install_incomplete", version: "0.3.0" });

    const banner = wrapper.find(".update-banner-warn");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("上次升级未完成");
    expect(banner.text()).toContain("v0.3.0");
  });
});
