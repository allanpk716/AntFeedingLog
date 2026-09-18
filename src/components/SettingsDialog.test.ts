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

// ── 数据页签日志区（数据安全二期票 01）──

async function openDataTab(logs?: { errors?: string[]; abnormal?: object | null }) {
  baseMock();
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
      case "get_recent_errors":
        return logs?.errors ?? [];
      case "get_last_abnormal_exit":
        return logs?.abnormal ?? null;
      default:
        return null;
    }
  });
  const wrapper = mount(SettingsDialog);
  await flushPromises();
  await wrapper.find(".tab-data").trigger("click");
  await flushPromises();
  return wrapper;
}

describe("设置弹窗「数据」页签日志区（票 01）", () => {
  it("「打开日志文件夹」按钮点击调 open_logs_folder（验收 7）", async () => {
    const wrapper = await openDataTab();

    const btn = wrapper.find(".open-logs-btn");
    expect(btn.exists()).toBe(true);
    invokeMock.mockClear();
    await btn.trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("open_logs_folder");
  });

  it("无错误无异常退出 → 显示「最近没有错误记录」，不出异常退出行", async () => {
    const wrapper = await openDataTab();

    expect(wrapper.find(".recent-errors-empty").text()).toContain("最近没有错误记录");
    expect(wrapper.find(".abnormal-exit").exists()).toBe(false);
  });

  it("有最近错误 → 摘要列表展示且新在上（验收 7）", async () => {
    const wrapper = await openDataTab({
      errors: [
        "[2026-09-18 10:02:00] [PANIC] 崩溃",
        "[2026-09-18 10:01:00] [ERROR] 错误甲",
      ],
    });

    const lines = wrapper.findAll(".recent-error-line");
    expect(lines.length).toBe(2);
    expect(lines[0].text()).toContain("崩溃");
    expect(lines[0].text()).not.toContain("错误甲");
    expect(wrapper.find(".recent-errors-empty").exists()).toBe(false);
  });

  it("上次异常退出（无原因）→ 展示时间与「疑强杀/断电」（验收 2 的展示面）", async () => {
    const wrapper = await openDataTab({
      abnormal: { session_started_at: "2026-09-18 08:00:00", reason: null },
    });

    const banner = wrapper.find(".abnormal-exit");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("上次异常退出");
    expect(banner.text()).toContain("2026-09-18 08:00:00");
    expect(banner.text()).toContain("无崩溃日志，疑强杀/断电");
  });

  it("上次异常退出（带 panic 原因）→ 原因行照展示（验收 1 的展示面）", async () => {
    const wrapper = await openDataTab({
      abnormal: {
        session_started_at: "2026-09-18 08:00:00",
        reason: "Rust panic: 库已损坏 @ db.rs:1",
      },
    });

    const banner = wrapper.find(".abnormal-exit");
    expect(banner.text()).toContain("上次异常退出");
    expect(banner.text()).toContain("Rust panic: 库已损坏 @ db.rs:1");
    expect(banner.text()).not.toContain("疑强杀");
  });
});
