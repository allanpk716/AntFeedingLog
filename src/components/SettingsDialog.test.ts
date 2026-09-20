import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import SettingsDialog from "./SettingsDialog.vue";
import type { BackupConfigInfo, CareActionItem, FoodItem, RestoreSummary, WebUiConfigInfo } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的
// invokeMock；事件订阅走 mock 工厂内置的立即退订空桩）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

// 轻提示接线（保湿方式+轻提示票 03）：toast 模块整体 mock 掉，断言调用点
// 弹没弹、弹的什么；store/宿主自身行为在 toast.test.ts / ToastHost.test.ts
const { showErrorMock, showSuccessMock } = vi.hoisted(() => ({
  showErrorMock: vi.fn(),
  showSuccessMock: vi.fn(),
}));
vi.mock("../lib/toast", () => ({
  showSuccess: showSuccessMock,
  showError: showErrorMock,
}));

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
  showErrorMock.mockClear();
  showSuccessMock.mockClear();
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

// ── 易腐配置 + 撤食行（票 01）──

function foodFixture(overrides: Partial<FoodItem> & { id: number }): FoodItem {
  return {
    name: `食物${overrides.id}`,
    enabled: true,
    sort: overrides.id,
    suggested_interval_days: 7,
    is_preset: true,
    referenced: false,
    ...overrides,
  };
}

function foodsFixture(): FoodItem[] {
  return [
    foodFixture({ id: 1, name: "种子", sort: 1, suggested_interval_days: 3, perishable: false, retrieval_hours: null }),
    foodFixture({ id: 2, name: "干虾仁", sort: 2, perishable: true, retrieval_hours: 24 }),
    foodFixture({ id: 3, name: "面包虫", sort: 3, perishable: false, retrieval_hours: null }),
  ];
}

function actionsFixture(): CareActionItem[] {
  return [
    { id: 1, name: "喂食", icon: null, kind: "reminding", is_feeding: true, suggested_interval_days: 3, enabled: true, sort: 1, is_preset: true, referenced: false },
    { id: 5, name: "撤食", icon: null, kind: "follow", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 2, is_preset: true, referenced: false },
    { id: 2, name: "活动区换水", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: true, sort: 3, is_preset: true, referenced: false },
  ];
}

function dictMock(cmd: string, foods: FoodItem[], actions: CareActionItem[]) {
  switch (cmd) {
    case "list_foods":
      return foods;
    case "list_actions":
      return actions;
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
    default:
      return null;
  }
}

async function openDictTab(tab: "foods" | "actions", foods: FoodItem[], actions: CareActionItem[]) {
  baseMock();
  invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foods, actions));
  const wrapper = mount(SettingsDialog);
  await flushPromises();
  await wrapper.find(tab === "foods" ? ".tab-foods" : ".tab-actions").trigger("click");
  await flushPromises();
  return wrapper;
}

function rowsOf(wrapper: VueWrapper) {
  return wrapper.findAll(".tab-body .dict-row");
}

describe("设置弹窗·食物易腐配置（票 01）", () => {
  it("存量易腐回显：干虾仁勾选且间隔 24 可编辑；未开易腐的间隔输入置灰（验收 4 后半）", async () => {
    const wrapper = await openDictTab("foods", foodsFixture(), actionsFixture());
    const rows = rowsOf(wrapper);

    const dry = rows[1]!;
    expect((dry.find(".perish-input").element as HTMLInputElement).checked).toBe(true);
    const dryHours = dry.find(".retrieval-hours-input").element as HTMLInputElement;
    expect(dryHours.value).toBe("24");
    expect(dryHours.disabled).toBe(false);

    const seed = rows[0]!;
    expect((seed.find(".perish-input").element as HTMLInputElement).checked).toBe(false);
    expect((seed.find(".retrieval-hours-input").element as HTMLInputElement).disabled).toBe(true);
  });

  it("开易腐+24 保存 → save_food 入参带 perishable=true / retrieval_hours=24（验收 4 通过分支）", async () => {
    const wrapper = await openDictTab("foods", foodsFixture(), actionsFixture());
    const rows = rowsOf(wrapper);

    await rows[2]!.find(".perish-input").setValue(true);
    await rows[2]!.find(".retrieval-hours-input").setValue("24");

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foodsFixture(), actionsFixture()));
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_food", {
      input: { id: 3, name: "面包虫", sort: 2, suggested_interval_days: 7, perishable: true, retrieval_hours: 24 },
    });
  });

  it("校验矩阵：开易腐+空/0/负/非整数/169 → 提示撤食间隔错误且不落库（验收 4 拒收分支）", async () => {
    for (const bad of ["", "0", "-3", "abc", "169"]) {
      const wrapper = await openDictTab("foods", foodsFixture(), actionsFixture());
      const rows = rowsOf(wrapper);
      await rows[2]!.find(".perish-input").setValue(true);
      await rows[2]!.find(".retrieval-hours-input").setValue(bad);

      invokeMock.mockClear();
      await wrapper.find(".tab-body .btn.primary").trigger("click");
      await flushPromises();

      expect(wrapper.find(".form-error").text()).toContain("撤食间隔");
      expect(invokeMock).not.toHaveBeenCalledWith("save_food", expect.anything());
    }
  });

  it("关易腐 → 间隔自动清空并置灰，保存入参 retrieval_hours=null（F5）", async () => {
    const wrapper = await openDictTab("foods", foodsFixture(), actionsFixture());
    const rows = rowsOf(wrapper);
    const dry = rows[1]!;

    await dry.find(".perish-input").setValue(false);
    const hours = dry.find(".retrieval-hours-input").element as HTMLInputElement;
    expect(hours.value).toBe("");
    expect(hours.disabled).toBe(true);

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foodsFixture(), actionsFixture()));
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_food", {
      input: { id: 2, name: "干虾仁", sort: 1, suggested_interval_days: 7, perishable: false, retrieval_hours: null },
    });
  });
});

describe("设置弹窗·操作页签撤食行（票 01）", () => {
  it("撤食行固定「跟随喂食」标注：无性质切换、无间隔编辑（验收 5）", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());
    const retrieval = rowsOf(wrapper)[1]!;

    expect(retrieval.find(".name-input").element as HTMLInputElement).toBeTruthy();
    expect((retrieval.find(".name-input").element as HTMLInputElement).value).toBe("撤食");
    expect(retrieval.find(".follow-chip").text()).toContain("跟随喂食");
    expect(retrieval.find(".kind-select").exists()).toBe(false);
    expect(retrieval.find(".interval-input").exists()).toBe(false);

    // 对照：普通提醒类行仍有性质切换与间隔
    const feed = rowsOf(wrapper)[0]!;
    expect(feed.find(".kind-select").exists()).toBe(true);
    expect(feed.find(".interval-input").exists()).toBe(true);
  });

  it("撤食行可停用：停用按钮走 set_action_enabled（停用=功能关闭）", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());
    const retrieval = rowsOf(wrapper)[1]!;

    invokeMock.mockClear();
    await retrieval.find(".row-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_action_enabled", { id: 5, enabled: false });
  });
});

// ── 通知页签「撤食提醒」开关（票 05）──

function retrievalSettingsFixture(overrides: Record<string, unknown> = {}) {
  return {
    notify_master_enabled: true,
    notify_overdue_enabled: true,
    notify_hibernation_enabled: true,
    notify_retrieval_enabled: true,
    wake_remind_days_ahead: 7,
    autostart_enabled: true,
    ...overrides,
  };
}

async function openNotifyTabWithSettings(settings: object, saved?: object) {
  baseMock();
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_actions":
      case "list_foods":
      case "list_locations":
        return [];
      case "get_settings":
        return settings;
      case "set_settings":
        return saved ?? settings;
      default:
        return null;
    }
  });
  const wrapper = mount(SettingsDialog);
  await flushPromises();
  await wrapper.find(".tab-notify").trigger("click");
  await flushPromises();
  return wrapper;
}

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

describe("设置弹窗·通知页签撤食提醒开关（票 05）", () => {
  it("开关存在且回显设置值：notify_retrieval_enabled=false → 未勾选", async () => {
    const wrapper = await openNotifyTabWithSettings(retrievalSettingsFixture({ notify_retrieval_enabled: false }));

    const box = wrapper.find(".retrieval-input").element as HTMLInputElement;
    expect(wrapper.find(".retrieval-input").exists()).toBe(true);
    expect(box.checked).toBe(false);
  });

  it("缺省（旧数据无该键）→ 兜底为开（默认开语义）", async () => {
    const legacy = settingsFixture();
    delete (legacy as Record<string, unknown>).notify_retrieval_enabled;
    const wrapper = await openNotifyTabWithSettings(legacy);

    expect((wrapper.find(".retrieval-input").element as HTMLInputElement).checked).toBe(true);
  });

  it("保存：入参带 notify_retrieval_enabled，开关状态原样落库（总开关关仍全静默是 Rust 侧语义）", async () => {
    const wrapper = await openNotifyTabWithSettings(retrievalSettingsFixture());

    await wrapper.find(".retrieval-input").setValue(false);
    invokeMock.mockClear();
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_settings", {
      input: retrievalSettingsFixture({ notify_retrieval_enabled: false }),
    });
    expect(wrapper.find(".saved-hint").text()).toContain("已保存");
  });
});

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

// ── 轻提示接线（保湿方式+轻提示票 03）──
// 判定原则（CLAUDE.md「操作反馈规范」）：操作完成后界面没有天然反馈的写操作
// 必须弹；字段级校验错误维持内联；非校验类失败走失败轻提示（error.value 通道
// 迁移）。审计结论（接线/不接的理由都写在这里）：
// - 接：字典「保存」（原成功静默）、行级停用/启用/删除（原成功静默）、
//   安全备份/导出 CSV/JSON/清理孤儿照片（导出走后端落文件，无浏览器下载类
//   即时可见反馈，弹窗内只有一行结果回显——按判定原则接）、弹窗加载失败。
// - 不接：「发送测试通知」（双通道回显即天然反馈）、通知/自动备份区「保存」
//   （自身有「已保存」回显）、恢复（摘要+结果回显+重拉）、打开文件夹类
//   （OS 层有反馈）；打卡/删除记录（关窗/列表即变）不在本弹窗，本票未触碰。

function toastActionsFixture(): CareActionItem[] {
  return [
    ...actionsFixture(),
    { id: 8, name: "已停操作", icon: null, kind: "log_only", is_feeding: false, suggested_interval_days: null, enabled: false, sort: 8, is_preset: false, referenced: false },
    { id: 9, name: "自定义操作", icon: null, kind: "reminding", is_feeding: false, suggested_interval_days: 2, enabled: true, sort: 9, is_preset: false, referenced: false },
  ];
}

function toastFoodsFixture(): FoodItem[] {
  return [
    ...foodsFixture(),
    foodFixture({ id: 8, name: "已停食物", enabled: false, sort: 8, is_preset: false, referenced: false, perishable: false, retrieval_hours: null }),
    foodFixture({ id: 9, name: "自定义食物", enabled: true, sort: 9, is_preset: false, referenced: false, perishable: false, retrieval_hours: null }),
  ];
}

async function openDataTabForToast(
  extra: {
    backupTo?: string;
    backupError?: string;
    exportTo?: string;
    exportError?: string;
    cleanOutcome?: { removed_dirs: number; freed_bytes: number; errors: string[] };
    cleanError?: string;
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
        return settingsFixture();
      case "get_recent_errors":
        return [];
      case "get_last_abnormal_exit":
        return null;
      case "list_orphan_photos":
        return orphanFixture();
      case "clean_orphan_photos":
        if (extra.cleanError) throw extra.cleanError;
        return extra.cleanOutcome ?? { removed_dirs: 0, freed_bytes: 0, errors: [] };
      case "backup_to":
        if (extra.backupError) throw extra.backupError;
        return extra.backupTo ?? "D:\\ant-bk\\manual-20260920.db";
      case "export_data":
        if (extra.exportError) throw extra.exportError;
        return extra.exportTo ?? "D:\\exports\\logs.csv";
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

describe("设置弹窗轻提示接线（保湿方式+轻提示票 03）", () => {
  it("操作「保存」成功 → 弹「已保存」成功轻提示（原成功静默就是要接的点）", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foodsFixture(), actionsFixture()));
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(showSuccessMock).toHaveBeenCalledWith("已保存");
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(wrapper.find(".form-error").exists()).toBe(false);
  });

  it("操作「保存」失败 → 失败轻提示带原因，旧内联失败红字不再出现", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "save_action") throw "库被锁住";
      return dictMock(cmd, foodsFixture(), actionsFixture());
    });
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("保存失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.find(".form-error").exists()).toBe(false);
  });

  it("字段级校验错误仍走内联红字，不进轻提示（F3 例外）", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());

    await rowsOf(wrapper)[0]!.find(".name-input").setValue("");
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();

    expect(wrapper.find(".form-error").text()).toContain("操作名字不能为空");
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(showSuccessMock).not.toHaveBeenCalled();
  });

  it("食物「保存」成功 → 弹「已保存」；失败 → 失败轻提示带原因", async () => {
    const wrapper = await openDictTab("foods", foodsFixture(), actionsFixture());

    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foodsFixture(), actionsFixture()));
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();
    expect(showSuccessMock).toHaveBeenCalledWith("已保存");

    showSuccessMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "save_food") throw "库被锁住";
      return dictMock(cmd, foodsFixture(), actionsFixture());
    });
    await wrapper.find(".tab-body .btn.primary").trigger("click");
    await flushPromises();
    expect(showErrorMock).toHaveBeenCalledWith("保存失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
  });

  it("操作行级 停用/启用/删除 成功各弹成功轻提示", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), toastActionsFixture());
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, foodsFixture(), toastActionsFixture()));

    // 停用喂食（id=1）
    await rowsOf(wrapper)[0]!.find(".row-btn").trigger("click");
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已停用「喂食」");

    // 启用已停操作（id=8）
    await rowsOf(wrapper)[3]!.find(".row-btn").trigger("click");
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已启用「已停操作」");

    // 删除自定义操作（id=9，非预置未被引用）
    await rowsOf(wrapper)[4]!.find(".erase-btn").trigger("click");
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已删除「自定义操作」");
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("操作行级 停用/启用/删除 失败各弹失败轻提示带原因，旧内联红字不再出现", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), toastActionsFixture());
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "set_action_enabled" || cmd === "erase_action") throw "库被锁住";
      return dictMock(cmd, foodsFixture(), toastActionsFixture());
    });

    await rowsOf(wrapper)[0]!.find(".row-btn").trigger("click"); // 停用喂食
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("停用失败", "库被锁住");

    await rowsOf(wrapper)[3]!.find(".row-btn").trigger("click"); // 启用已停操作
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("启用失败", "库被锁住");

    await rowsOf(wrapper)[4]!.find(".erase-btn").trigger("click"); // 删除自定义操作
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("删除失败", "库被锁住");
    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(wrapper.find(".form-error").exists()).toBe(false);
  });

  it("食物行级 停用/启用/删除 成功各弹成功轻提示", async () => {
    const wrapper = await openDictTab("foods", toastFoodsFixture(), actionsFixture());
    invokeMock.mockImplementation(async (cmd: string) => dictMock(cmd, toastFoodsFixture(), actionsFixture()));

    await rowsOf(wrapper)[0]!.find(".row-btn").trigger("click"); // 停用种子
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已停用「种子」");

    await rowsOf(wrapper)[3]!.find(".row-btn").trigger("click"); // 启用已停食物
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已启用「已停食物」");

    await rowsOf(wrapper)[4]!.find(".erase-btn").trigger("click"); // 删除自定义食物
    await flushPromises();
    expect(showSuccessMock).toHaveBeenLastCalledWith("已删除「自定义食物」");
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("食物行级 停用/启用/删除 失败各弹失败轻提示带原因", async () => {
    const wrapper = await openDictTab("foods", toastFoodsFixture(), actionsFixture());
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "set_food_enabled" || cmd === "erase_food") throw "库被锁住";
      return dictMock(cmd, toastFoodsFixture(), actionsFixture());
    });

    await rowsOf(wrapper)[0]!.find(".row-btn").trigger("click"); // 停用种子
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("停用失败", "库被锁住");

    await rowsOf(wrapper)[3]!.find(".row-btn").trigger("click"); // 启用已停食物
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("启用失败", "库被锁住");

    await rowsOf(wrapper)[4]!.find(".erase-btn").trigger("click"); // 删除自定义食物
    await flushPromises();
    expect(showErrorMock).toHaveBeenLastCalledWith("删除失败", "库被锁住");
  });

  it("安全备份成功 → 成功轻提示，路径结果回显保留（toast 给即时反馈，路径留在回显里）", async () => {
    const wrapper = await openDataTabForToast({ backupTo: "D:\\ant-bk\\manual-20260920.db" });

    await wrapper.find(".backup-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("backup_to");
    expect(showSuccessMock).toHaveBeenCalledWith("已备份");
    expect(wrapper.find(".data-result").text()).toContain("已备份到：D:\\ant-bk\\manual-20260920.db");
  });

  it("安全备份失败 → 失败轻提示带原因", async () => {
    const wrapper = await openDataTabForToast({ backupError: "磁盘没有空间" });

    await wrapper.find(".backup-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("备份失败", "磁盘没有空间");
    expect(showSuccessMock).not.toHaveBeenCalled();
  });

  it("导出 CSV / JSON 成功 → 各弹成功轻提示（后端落文件，无浏览器下载类即时反馈，故接线）", async () => {
    const wrapper = await openDataTabForToast({ exportTo: "D:\\exports\\logs.csv" });

    await wrapper.find(".export-csv-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("export_data", { format: "csv" });
    expect(showSuccessMock).toHaveBeenCalledWith("已导出 CSV");

    await wrapper.find(".export-json-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("export_data", { format: "json" });
    expect(showSuccessMock).toHaveBeenCalledWith("已导出 JSON");
    expect(showErrorMock).not.toHaveBeenCalled();
  });

  it("导出失败 → 失败轻提示带原因", async () => {
    const wrapper = await openDataTabForToast({ exportError: "导出目录不可写" });

    await wrapper.find(".export-csv-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("导出失败", "导出目录不可写");
  });

  it("清理孤儿照片成功（两段确认后）→ 成功轻提示，数量/释放回显保留", async () => {
    const wrapper = await openDataTabForToast({
      cleanOutcome: { removed_dirs: 1, freed_bytes: 15 * 1024 * 1024, errors: [] },
    });

    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();

    expect(showSuccessMock).toHaveBeenCalledWith("孤儿照片已清理");
    expect(wrapper.find(".orphan-result").text()).toContain("已清理 1 个隔离目录");
  });

  it("清理孤儿照片失败 → 失败轻提示带原因", async () => {
    const wrapper = await openDataTabForToast({ cleanError: "删除隔离目录失败" });

    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".orphan-clean-btn").trigger("click");
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("清理失败", "删除隔离目录失败");
  });

  it("判定原则审计：「发送测试通知」有双通道回显即天然反馈，不接轻提示", async () => {
    const wrapper = await openNotifyTab({ status: { source: "env", configured: true } });

    invokeMock.mockClear();
    invokeMock.mockResolvedValueOnce({ desktop_ok: true, desktop_error: null, pushover: { ok: true, error: null } });
    await wrapper.find(".tab-body .dlg-btns .btn:not(.primary)").trigger("click");
    await flushPromises();

    expect(showSuccessMock).not.toHaveBeenCalled();
    expect(showErrorMock).not.toHaveBeenCalled();
    expect(wrapper.find(".saved-hint").text()).toContain("手机 ✓");
  });

  it("弹窗加载失败 → 失败轻提示带原因，不再走内联红字（非校验失败通道迁移）", async () => {
    baseMock();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_actions") throw "库打不开";
      switch (cmd) {
        case "list_foods":
        case "list_locations":
          return [];
        case "get_settings":
          return settingsFixture();
        default:
          return null;
      }
    });
    const wrapper = mount(SettingsDialog);
    await flushPromises();

    expect(showErrorMock).toHaveBeenCalledWith("设置加载失败", "库打不开");
    expect(wrapper.find(".form-error").exists()).toBe(false);
  });

  it("操作 tab 顶部常驻提示行：每窝可单独设周期且优先于全局设置", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());

    const hint = wrapper.find(".per-colony-hint");
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toContain("单独设「每窝周期」");
    expect(hint.text()).toContain("优先于此处的全局设置");
  });

  it("「仅登记」下拉 tooltip 补全：含单个窝仍可在窝编辑里设周期提醒", async () => {
    const wrapper = await openDictTab("actions", foodsFixture(), actionsFixture());

    // 活动区换水是 log_only 行，走「仅登记」分支标题
    const select = rowsOf(wrapper)[2]!.find(".kind-select");
    expect(select.exists()).toBe(true);
    expect(select.attributes("title")).toContain("只记录，本页不催促");
    expect(select.attributes("title")).toContain("单个窝仍可在窝编辑里设周期提醒");
  });
});
