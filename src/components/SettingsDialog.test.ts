import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import SettingsDialog from "./SettingsDialog.vue";
import type { BackupConfigInfo } from "../types";

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

// ── 数据页签自动备份区（数据安全二期票 02）──

function backupConfigFixture(overrides: Partial<BackupConfigInfo> = {}): BackupConfigInfo {
  return {
    enabled: true,
    backup_dir: null,
    keep_count: 30,
    last_backup_date: null,
    last_data_write_date: null,
    last_result: null,
    ...overrides,
  };
}

async function openDataTabWithBackup(
  config: BackupConfigInfo,
  extra: { pickResult?: string | null; saved?: BackupConfigInfo } = {},
) {
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
        return [];
      case "get_last_abnormal_exit":
        return null;
      case "get_backup_config":
        return config;
      case "pick_backup_dir":
        return extra.pickResult ?? null;
      case "set_backup_config":
        return extra.saved ?? config;
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

describe("设置弹窗「数据」页签自动备份区（数据安全二期票 02）", () => {
  it("加载默认配置：开关勾选、目录显示未设置、保留份数回显 30（验收 1 展示面）", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture());

    expect(wrapper.find(".backup-section").exists()).toBe(true);
    expect((wrapper.find(".auto-enabled-input").element as HTMLInputElement).checked).toBe(true);
    expect(wrapper.find(".backup-dir-current").text()).toContain("未设置");
    expect((wrapper.find(".keep-count-input").element as HTMLInputElement).value).toBe("30");
    expect(wrapper.find(".backup-last-status").text()).toContain("尚未备份");
  });

  it("开关开+目录未设 → 显示「未设置备份目录，自动备份未生效」（验收 3）", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture());

    const hint = wrapper.find(".backup-inactive");
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toContain("未设置备份目录，自动备份未生效");
  });

  it("目录已设 → 未生效提示消失，显示当前目录", async () => {
    const wrapper = await openDataTabWithBackup(
      backupConfigFixture({ backup_dir: "D:\\ant-bk" }),
    );

    expect(wrapper.find(".backup-inactive").exists()).toBe(false);
    expect(wrapper.find(".backup-dir-current").text()).toContain("D:\\ant-bk");
  });

  it("选择目录：走 pick_backup_dir，选完显示所选目录（验收 2）", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), {
      pickResult: "D:\\ant-bk",
    });

    invokeMock.mockClear();
    await wrapper.find(".pick-backup-dir-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("pick_backup_dir");
    expect(wrapper.find(".backup-dir-current").text()).toContain("D:\\ant-bk");
  });

  it("选择目录：用户取消（null）→ 目录不变", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), { pickResult: null });

    await wrapper.find(".pick-backup-dir-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".backup-dir-current").text()).toContain("未设置");
  });

  it("保存：合法配置按三项入参落库并回显已保存（验收 1 持久化入口）", async () => {
    const saved = backupConfigFixture({ backup_dir: "D:\\ant-bk" });
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), {
      pickResult: "D:\\ant-bk",
      saved,
    });

    await wrapper.find(".pick-backup-dir-btn").trigger("click");
    await flushPromises();
    invokeMock.mockClear();
    await wrapper.find(".save-backup-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_backup_config", {
      input: { enabled: true, backup_dir: "D:\\ant-bk", keep_count: 30 },
    });
    expect(wrapper.find(".backup-saved-hint").text()).toContain("已保存");
  });

  it("保存：设了目录后未生效提示消失（验收 3 后半）", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), {
      pickResult: "D:\\ant-bk",
      saved: backupConfigFixture({ backup_dir: "D:\\ant-bk" }),
    });
    expect(wrapper.find(".backup-inactive").exists()).toBe(true);

    await wrapper.find(".save-backup-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".backup-inactive").exists()).toBe(false);
  });

  it("保留份数非法（0/366/空）→ 提示错误且不落库（验收 4）", async () => {
    for (const bad of ["0", "366", ""]) {
      const wrapper = await openDataTabWithBackup(backupConfigFixture());
      invokeMock.mockClear();
      await wrapper.find(".keep-count-input").setValue(bad);
      await wrapper.find(".save-backup-btn").trigger("click");
      await flushPromises();

      expect(wrapper.find(".backup-error").text()).toContain("1–365");
      expect(invokeMock).not.toHaveBeenCalledWith("set_backup_config", expect.anything());
    }
  });

  it("保留份数改为 90 → 落库入参带 90（验收 4 前半：可改）", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), {
      saved: backupConfigFixture({ keep_count: 90 }),
    });

    invokeMock.mockClear();
    await wrapper.find(".keep-count-input").setValue("90");
    await wrapper.find(".save-backup-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_backup_config", {
      input: { enabled: true, backup_dir: null, keep_count: 90 },
    });
    expect((wrapper.find(".keep-count-input").element as HTMLInputElement).value).toBe("90");
  });

  it("开关关掉后保存 → 入参 enabled=false", async () => {
    const wrapper = await openDataTabWithBackup(backupConfigFixture(), {
      saved: backupConfigFixture({ enabled: false }),
    });

    invokeMock.mockClear();
    await wrapper.find(".auto-enabled-input").setValue(false);
    await wrapper.find(".save-backup-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_backup_config", {
      input: { enabled: false, backup_dir: null, keep_count: 30 },
    });
  });

  it("上次备份状态四态之一：成功+时间（验收 5）", async () => {
    const wrapper = await openDataTabWithBackup(
      backupConfigFixture({ last_result: { ok: true, at: "2026-09-18 08:00:00", reason: null } }),
    );

    const status = wrapper.find(".backup-last-status");
    expect(status.text()).toContain("成功");
    expect(status.text()).toContain("2026-09-18 08:00:00");
  });

  it("上次备份状态四态之一：失败+时间+原因（验收 5，本票以手工置入状态验证展示）", async () => {
    const wrapper = await openDataTabWithBackup(
      backupConfigFixture({
        last_result: { ok: false, at: "2026-09-18 09:00:00", reason: "网盘掉线" },
      }),
    );

    const status = wrapper.find(".backup-last-status");
    expect(status.text()).toContain("失败");
    expect(status.text()).toContain("2026-09-18 09:00:00");
    expect(status.text()).toContain("网盘掉线");
  });
});
