import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import SettingsDialog from "./SettingsDialog.vue";
import type { BackupConfigInfo, RestoreSummary, WebUiConfigInfo } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的
// invokeMock；事件订阅走 mock 工厂内置的立即退订空桩）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

function baseMock(updateState: object = { status: "idle" }) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_actions":
      case "list_foods":
      case "list_locations":
        return [];
      case "get_settings":
        return settingsFixture();
      case "pushover_status":
        return { source: "none", configured: false };
      case "get_app_version":
        return "0.1.0";
      case "get_update_state":
        return updateState;
      default:
        return null;
    }
  });
}

/** 票 11：设置回放统一带凭据两键（覆盖处用参数换值） */
function settingsFixture(overrides: Partial<{ pushover_user: string; pushover_token: string }> = {}) {
  return {
    notify_master_enabled: true,
    notify_overdue_enabled: true,
    notify_hibernation_enabled: true,
    wake_remind_days_ahead: 7,
    autostart_enabled: true,
    pushover_user: "",
    pushover_token: "",
    ...overrides,
  };
}

// ── 通知 tab Pushover 应用内配置（webui-checkin 票 11）──

function notifyTabMock(
  opts: {
    status?: { source: string; configured: boolean };
    pushover_user?: string;
    pushover_token?: string;
    saved?: object;
  } = {},
) {
  baseMock();
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_actions":
      case "list_foods":
      case "list_locations":
        return [];
      case "get_settings":
        return settingsFixture({ pushover_user: opts.pushover_user ?? "", pushover_token: opts.pushover_token ?? "" });
      case "pushover_status":
        return opts.status ?? { source: "none", configured: false };
      case "set_settings":
        return opts.saved ?? settingsFixture({ pushover_user: opts.pushover_user ?? "", pushover_token: opts.pushover_token ?? "" });
      case "get_app_version":
        return "0.1.0";
      case "get_update_state":
        return { status: "idle" };
      default:
        return null;
    }
  });
}

async function openNotifyTab(opts?: Parameters<typeof notifyTabMock>[0]) {
  notifyTabMock(opts);
  const wrapper = mount(SettingsDialog);
  await flushPromises();
  await wrapper.find(".tab-notify").trigger("click");
  await flushPromises();
  return wrapper;
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
});

describe("设置弹窗通知 tab Pushover 应用内配置（webui-checkin 票 11）", () => {
  it("生效来源三态标注：应用内配置 / 系统环境变量 / 未配置（验收 2 后半）", async () => {
    const cases = [
      { status: { source: "app", configured: true }, label: "应用内配置" },
      { status: { source: "env", configured: true }, label: "系统环境变量" },
      { status: { source: "none", configured: false }, label: "未配置" },
    ];
    for (const c of cases) {
      const wrapper = await openNotifyTab({ status: c.status });
      const label = wrapper.find(".pushover-source-label");
      expect(label.exists()).toBe(true);
      expect(label.text()).toContain(c.label);
      wrapper.unmount();
    }
  });

  it("两输入框默认打码（type=password），「查看明文」切换明文、可再隐藏（验收 2 前半）", async () => {
    const wrapper = await openNotifyTab({ pushover_user: "u-应用内", pushover_token: "t-令牌" });

    const user = wrapper.find(".pushover-user-input");
    const token = wrapper.find(".pushover-token-input");
    expect((user.element as HTMLInputElement).type).toBe("password");
    expect((token.element as HTMLInputElement).type).toBe("password");
    expect((user.element as HTMLInputElement).value).toBe("u-应用内");
    expect((token.element as HTMLInputElement).value).toBe("t-令牌");

    await wrapper.find(".pushover-reveal-user-btn").trigger("click");
    await wrapper.find(".pushover-reveal-token-btn").trigger("click");
    expect((user.element as HTMLInputElement).type).toBe("text");
    expect((token.element as HTMLInputElement).type).toBe("text");
    expect(wrapper.find(".pushover-reveal-user-btn").text()).toContain("隐藏");

    await wrapper.find(".pushover-reveal-user-btn").trigger("click");
    expect((user.element as HTMLInputElement).type).toBe("password");
  });

  it("占位符提示「留空则使用系统环境变量」（回落语义上屏）", async () => {
    const wrapper = await openNotifyTab();

    expect((wrapper.find(".pushover-user-input").element as HTMLInputElement).placeholder).toContain("留空");
    expect((wrapper.find(".pushover-user-input").element as HTMLInputElement).placeholder).toContain("环境变量");
    expect((wrapper.find(".pushover-token-input").element as HTMLInputElement).placeholder).toContain("环境变量");
  });

  it("明文入库并随备份扩散的风险提示固定展示（验收 4）", async () => {
    const wrapper = await openNotifyTab();

    const hint = wrapper.find(".pushover-risk-hint");
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toContain("明文");
    expect(hint.text()).toContain("备份");
  });

  it("保存：凭据两值随 set_settings 落库，保存后重查生效来源（验收 3 的持久化入口）", async () => {
    const wrapper = await openNotifyTab();

    await wrapper.find(".pushover-user-input").setValue("u-新值");
    await wrapper.find(".pushover-token-input").setValue("t-新值");
    invokeMock.mockClear();
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith(
      "set_settings",
      expect.objectContaining({
        input: expect.objectContaining({ pushover_user: "u-新值", pushover_token: "t-新值" }),
      }),
    );
    // 保存后生效来源可能切换：pushover_status 至少被重查一次
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "pushover_status")).toBe(true);
    expect(wrapper.find(".saved-hint").text()).toContain("已保存");
  });

  it("「发送测试通知」按钮在通知 tab 照旧可用（验收 3：测的就是当前生效来源）", async () => {
    const wrapper = await openNotifyTab({ status: { source: "env", configured: true } });

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce({ desktop_ok: true, desktop_error: null, pushover: { ok: true, error: null } });
    await wrapper.find(".tab-body .dlg-btns .btn:not(.primary)").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("send_test_notification");
    expect(wrapper.find(".saved-hint").text()).toContain("手机 ✓");
  });
});

describe("设置弹窗「更新」节（票 06）", () => {  it("新增「更新」tab：点开显示当前版本与检查入口（验收 1 的挂载面）", async () => {
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

// ── 网页端 tab（webui-checkin 票 03）：挂载面接线；面板细节在 WebUiPanel.test.ts ──

function webUiConfigFixture(): WebUiConfigInfo {
  return {
    enabled: false,
    segments: [],
    port: 17321,
    token: "0123456789abcdef0123456789abcdef",
    token_generated_at: "2026-09-19 08:00:00",
  };
}

describe("设置弹窗「网页端」tab（webui-checkin 票 03）", () => {
  it("新增「网页端」tab：点开渲染面板（总开关/网段/端口/凭证/地址），面板加载走新命令", async () => {
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
        case "get_webui_config":
          return webUiConfigFixture();
        case "list_network_segments":
          return [{ cidr: "100.84.0.0/16", encrypted_mesh: true, label: "NetBird 虚拟网" }];
        case "get_access_url":
          return "http://100.84.12.3:17321/#token=0123456789abcdef0123456789abcdef";
        default:
          return null;
      }
    });
    const wrapper = mount(SettingsDialog);
    await flushPromises();
    await wrapper.find(".tab-webui").trigger("click");
    await flushPromises();

    expect(wrapper.find(".webui-panel").exists()).toBe(true);
    expect((wrapper.find(".webui-enabled-input").element as HTMLInputElement).checked).toBe(false);
    expect(wrapper.findAll(".seg-row")[0].text()).toContain("NetBird 虚拟网");
    expect(wrapper.find(".token-masked").exists()).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("get_webui_config");
    expect(invokeMock).toHaveBeenCalledWith("list_network_segments");
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

// ── 数据页签恢复区（数据安全二期票 04）──

function restoreSummaryFixture(overrides: Partial<RestoreSummary> = {}): RestoreSummary {
  return {
    backup_date: "2026-09-10",
    colony_count: 2,
    log_count: 5,
    backup_dir_in_backup: null,
    photo_count: 0,
    ...overrides,
  };
}

async function openDataTabForRestore(
  extra: {
    pickResult?: string | null;
    preview?: RestoreSummary;
    previewError?: string;
    apply?: string;
    applyError?: string;
    withBackupDir?: boolean;
  } = {},
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
        return backupConfigFixture({ backup_dir: extra.withBackupDir ? "D:\\ant-bk" : null });
      case "pick_restore_file":
        return extra.pickResult ?? null;
      case "restore_preview":
        if (extra.previewError) throw extra.previewError;
        return extra.preview ?? restoreSummaryFixture();
      case "restore_apply":
        if (extra.applyError) throw extra.applyError;
        return extra.apply ?? "done";
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

async function pickAndPreview(wrapper: ReturnType<typeof mount>) {
  await wrapper.find(".restore-btn").trigger("click");
  await flushPromises();
}

describe("设置弹窗「数据」页签恢复区（数据安全二期票 04）", () => {
  it("恢复按钮存在，且位于自动备份区之后、手动按钮区之前（D11 位置）", async () => {
    const wrapper = await openDataTabForRestore();

    const section = wrapper.find(".restore-section");
    expect(section.exists()).toBe(true);
    expect(wrapper.find(".restore-btn").exists()).toBe(true);
    // DOM 顺序：自动备份区 → 恢复区 → 手动按钮区
    const html = wrapper.find(".tab-body").element.innerHTML;
    const autoPos = html.indexOf("save-backup-btn");
    const restorePos = html.indexOf("restore-btn");
    const manualPos = html.indexOf("reveal-btn");
    expect(autoPos).toBeGreaterThan(-1);
    expect(restorePos).toBeGreaterThan(autoPos);
    expect(manualPos).toBeGreaterThan(restorePos);
  });

  it("选完文件调 restore_preview，摘要展示备份日期/窝数/记录数/照片/备份内目录值 + 固定文案（验收 1）", async () => {
    const wrapper = await openDataTabForRestore({
      pickResult: "D:\\ant-bk\\ant-feeding-log-backup-20260910-080000.db",
      preview: restoreSummaryFixture({ backup_dir_in_backup: "D:\\old-bk" }),
      withBackupDir: true,
    });

    invokeMock.mockClear();
    await pickAndPreview(wrapper);

    // 默认定位备份目录：pick 入参带配置里的目录
    expect(invokeMock).toHaveBeenCalledWith("pick_restore_file", { defaultDir: "D:\\ant-bk" });
    expect(invokeMock).toHaveBeenCalledWith("restore_preview", {
      path: "D:\\ant-bk\\ant-feeding-log-backup-20260910-080000.db",
    });
    const panel = wrapper.find(".restore-panel");
    expect(panel.exists()).toBe(true);
    const text = panel.text();
    expect(text).toContain("2026-09-10");
    expect(text).toContain("2");
    expect(text).toContain("5");
    expect(text).toContain("D:\\old-bk");
    // 票 10：裸库恢复摘要标注不含照片
    expect(text).toContain("不含照片");
    expect(text).toContain("恢复将整体替换当前全部数据");
    expect(text).toContain("备份设置保持当前值，不随恢复回滚");
  });

  it("用户取消选文件 → 不调 restore_preview、不出摘要面板", async () => {
    const wrapper = await openDataTabForRestore({ pickResult: null });

    invokeMock.mockClear();
    await pickAndPreview(wrapper);

    expect(invokeMock).toHaveBeenCalledWith("pick_restore_file", expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith("restore_preview", expect.anything());
    expect(wrapper.find(".restore-panel").exists()).toBe(false);
  });

  it("preview 拒绝（损坏/未来版本等）→ 展示拒绝原因、无摘要面板（验收 2 的展示面）", async () => {
    const wrapper = await openDataTabForRestore({
      pickResult: "D:\\bk\\bad.db",
      previewError: "备份文件已损坏",
    });

    await pickAndPreview(wrapper);

    expect(wrapper.find(".restore-error").text()).toContain("备份文件已损坏");
    expect(wrapper.find(".restore-panel").exists()).toBe(false);
  });

  it("0 窝 0 条摘要 → 出选错文件疑点提示（D6 最后防线的界面面）", async () => {
    const wrapper = await openDataTabForRestore({
      pickResult: "D:\\bk\\wrong.db",
      preview: restoreSummaryFixture({ colony_count: 0, log_count: 0, backup_date: null }),
    });

    await pickAndPreview(wrapper);

    expect(wrapper.find(".restore-suspicious").text()).toContain("0 窝 0 条");
    expect(wrapper.find(".restore-suspicious").text()).toContain("选错了文件");
  });

  it("二段确认：第一次点只进入确认态，第二次才调 restore_apply（与删除记录同待遇）", async () => {
    const wrapper = await openDataTabForRestore({ pickResult: "D:\\bk\\good.db", apply: "done" });

    await pickAndPreview(wrapper);
    invokeMock.mockClear();
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).not.toHaveBeenCalledWith("restore_apply", expect.anything());
    expect(wrapper.find(".restore-confirm-btn").text()).toContain("再次点击确认恢复");

    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("restore_apply", { path: "D:\\bk\\good.db" });
    expect(wrapper.find(".restore-result").text()).toContain("恢复完成，界面已刷新");
    expect(wrapper.find(".restore-panel").exists()).toBe(false);
  });

  it("恢复成功 done → 弹窗内数据重拉自新库（list_actions 再次调用）并向外抛 changed", async () => {
    const wrapper = await openDataTabForRestore({ pickResult: "D:\\bk\\good.db", apply: "done" });

    await pickAndPreview(wrapper);
    invokeMock.mockClear();
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("list_actions");
    expect(wrapper.emitted("changed")).toBeTruthy();
  });

  it("恢复成功但需重启（done_needs_restart）→ 提示「请重启应用」（spec D5 附录 #10）", async () => {
    const wrapper = await openDataTabForRestore({
      pickResult: "D:\\bk\\good.db",
      apply: "done_needs_restart",
    });

    await pickAndPreview(wrapper);
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".restore-result").text()).toContain("重启");
  });

  it("restore_apply 失败 → 展示错误、不误报成功", async () => {
    const wrapper = await openDataTabForRestore({
      pickResult: "D:\\bk\\good.db",
      applyError: "恢复执行失败：库锁不可用",
    });

    await pickAndPreview(wrapper);
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".restore-confirm-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".restore-error").text()).toContain("库锁不可用");
    // 结果与错误区分展示，失败绝不误报成功文案
    expect(wrapper.find(".restore-result").exists()).toBe(false);
  });

  it("取消按钮收起摘要面板且不产生任何写命令", async () => {
    const wrapper = await openDataTabForRestore({ pickResult: "D:\\bk\\good.db" });

    await pickAndPreview(wrapper);
    invokeMock.mockClear();
    await wrapper.find(".restore-cancel-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".restore-panel").exists()).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith("restore_apply", expect.anything());
  });
});

// ── 数据页签巢况照片孤儿区（webui-checkin 票 07）──

function orphanFixture(overrides: { dir_count?: number; file_count?: number; total_bytes?: number } = {}) {
  return { dir_count: 1, file_count: 3, total_bytes: 15 * 1024 * 1024, ...overrides };
}

async function openDataTabWithOrphans(
  stats: { dir_count: number; file_count: number; total_bytes: number } | null,
  cleanOutcome?: { removed_dirs: number; freed_bytes: number; errors: string[] },
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
      case "list_orphan_photos":
        if (stats === null) throw "数据目录未初始化";
        return stats;
      case "clean_orphan_photos":
        return cleanOutcome ?? { removed_dirs: 0, freed_bytes: 0, errors: [] };
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

describe("设置弹窗「数据」页签巢况照片孤儿区（webui-checkin 票 07）", () => {
  it("有孤儿：展示隔离目录数/张数/占用，出「清理孤儿照片」按钮", async () => {
    const wrapper = await openDataTabWithOrphans(orphanFixture());

    expect(wrapper.find(".orphan-section").exists()).toBe(true);
    expect(wrapper.find(".orphan-stats-value").text()).toContain("1 个隔离目录");
    expect(wrapper.find(".orphan-stats-value").text()).toContain("共 3 张");
    expect(wrapper.find(".orphan-stats-value").text()).toContain("15.0 MB");
    expect(wrapper.find(".orphan-clean-btn").exists()).toBe(true);
    expect(wrapper.find(".orphan-clean-btn").text()).toBe("清理孤儿照片");
  });

  it("无孤儿：显示「无孤儿」，不出清理按钮", async () => {
    const wrapper = await openDataTabWithOrphans(orphanFixture({ dir_count: 0, file_count: 0, total_bytes: 0 }));

    expect(wrapper.find(".orphan-stats-value").text()).toContain("无孤儿");
    expect(wrapper.find(".orphan-clean-btn").exists()).toBe(false);
  });

  it("两段确认：第一次点只进入确认态，第二次才调 clean_orphan_photos 并重拉统计", async () => {
    const wrapper = await openDataTabWithOrphans(
      orphanFixture(),
      { removed_dirs: 1, freed_bytes: 15 * 1024 * 1024, errors: [] },
    );

    invokeMock.mockClear();
    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".orphan-clean-btn").text()).toBe("再次点击确认清理");
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "clean_orphan_photos")).toBe(false);

    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("clean_orphan_photos");
    expect(wrapper.find(".orphan-result").text()).toContain("已清理 1 个隔离目录");
    expect(wrapper.find(".orphan-result").text()).toContain("15.0 MB");
    // 清理完成后重拉统计（本 mock 仍返回有孤儿，只验证重拉发生）
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "list_orphan_photos").length,
    ).toBeGreaterThanOrEqual(1);
  });

  it("「重新统计」按钮重调 list_orphan_photos", async () => {
    const wrapper = await openDataTabWithOrphans(orphanFixture());

    invokeMock.mockClear();
    await wrapper.find(".orphan-refresh-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("list_orphan_photos");
  });

  it("统计读取失败（首载）静默不打扰，手动重试才报错", async () => {
    const wrapper = await openDataTabWithOrphans(null);

    expect(wrapper.find(".orphan-stats-value").text()).toContain("读取失败");
    expect(wrapper.find(".orphan-error").exists()).toBe(false);

    await wrapper.find(".orphan-refresh-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".orphan-error").text()).toContain("数据目录未初始化");
  });
});
