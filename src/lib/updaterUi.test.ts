import { describe, expect, it } from "vitest";
import {
  checkViewForOutcome,
  formatBytes,
  incompleteGuidance,
  installFailedText,
  installViewForOutcome,
  progressText,
  shouldRefreshProgress,
  stateBannerFor,
  toProgressView,
  upToDateText,
  updateAvailableTitle,
  type CheckOutcome,
  type DownloadProgress,
  type InstallOutcome,
  type ProgressView,
  type UpdateState,
} from "./updaterUi";

// ── 状态映射：检查结果三态（Rust CheckOutcome 契约，tag=status）──

describe("检查结果 → 展示视图", () => {
  it("up_to_date → 平静的「已是最新」", () => {
    const outcome: CheckOutcome = { status: "up_to_date" };
    const view = checkViewForOutcome(outcome);
    expect(view.kind).toBe("up_to_date");
    expect(view.kind === "up_to_date" && view.text).toBe(upToDateText());
    expect(upToDateText()).toContain("已是最新");
  });

  it("update_available → 版本号 + 说明（说明可缺）", () => {
    const withNotes = checkViewForOutcome({
      status: "update_available",
      version: "0.3.0",
      notes: "修复若干问题",
    });
    expect(withNotes).toEqual({ kind: "available", version: "0.3.0", notes: "修复若干问题" });
    expect(updateAvailableTitle("0.3.0")).toBe("发现新版本 v0.3.0");

    const noNotes = checkViewForOutcome({
      status: "update_available",
      version: "0.3.0",
      notes: null,
    });
    expect(noNotes).toEqual({ kind: "available", version: "0.3.0", notes: null });
  });

  it("检查失败（invoke reject 的 string）→ 一次性失败视图", () => {
    // 手动路径失败折为 Err（Rust manual_result），前端拿到 reject 字符串
    const view = checkViewForOutcome("检查更新失败: HTTP 404");
    expect(view).toEqual({ kind: "failed", message: "检查更新失败: HTTP 404" });
  });
});

// ── 状态映射：安装结果两态 ──

describe("安装结果 → 展示视图", () => {
  it("install_started → 「将重启以完成安装」", () => {
    const outcome: InstallOutcome = { status: "install_started", version: "0.3.0" };
    const view = installViewForOutcome(outcome);
    expect(view.kind).toBe("started");
    expect(view.kind === "started" && view.text).toContain("重启");
  });

  it("install_failed → 版本 + 原因，文案含重试/手动下载引导语义", () => {
    const outcome: InstallOutcome = {
      status: "install_failed",
      version: "0.3.0",
      message: "下载更新失败: 请求超时",
    };
    const view = installViewForOutcome(outcome);
    expect(view).toMatchObject({ kind: "failed", version: "0.3.0", message: "下载更新失败: 请求超时" });
    const text = installFailedText("0.3.0", "下载更新失败: 请求超时");
    expect(text).toContain("v0.3.0");
    expect(text).toContain("下载更新失败: 请求超时");
  });
});

// ── 状态映射：启动残留（get_update_state）→ 引导横幅 ──

describe("更新状态 → 引导横幅", () => {
  const states: UpdateState[] = [
    { status: "idle" },
    { status: "last_install_succeeded", version: "0.2.0" },
    { status: "last_install_incomplete", version: "0.3.0" },
  ];

  it("idle → 不出横幅", () => {
    expect(stateBannerFor(states[0])).toEqual({ kind: "none" });
  });

  it("last_install_succeeded → 「已升级到 vX」提示", () => {
    const banner = stateBannerFor(states[1]);
    expect(banner.kind).toBe("succeeded");
    expect(banner.kind === "succeeded" && banner.text).toContain("v0.2.0");
  });

  it("last_install_incomplete → 未完成引导文案，含目标版本与出口", () => {
    const banner = stateBannerFor(states[2]);
    expect(banner).toMatchObject({ kind: "incomplete", version: "0.3.0" });
    const text = incompleteGuidance("0.3.0");
    expect(text).toContain("上次升级未完成");
    expect(text).toContain("v0.3.0");
    expect(text).toContain("重试");
    expect(text).toContain("手动下载");
  });
});

// ── 进度节流：逐 chunk 无节流事件 → 展示层自己做节流 ──

describe("下载进度", () => {
  it("toProgressView 算百分比：total 缺失/为 0 → null，封顶 100", () => {
    expect(toProgressView({ downloaded: 250, total: 1000 })).toMatchObject({
      downloaded: 250,
      total: 1000,
      percent: 25,
    });
    expect(toProgressView({ downloaded: 250, total: null }).percent).toBeNull();
    expect(toProgressView({ downloaded: 250, total: 0 }).percent).toBeNull();
    expect(toProgressView({ downloaded: 1500, total: 1000 }).percent).toBe(100);
  });

  it("formatBytes：B / KB / MB / GB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(12_582_912)).toBe("12.0 MB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
  });

  it("progressText：有 total 带总量与百分比，无 total 只报已下载", () => {
    const withTotal: ProgressView = toProgressView({ downloaded: 250, total: 1000 });
    expect(progressText(withTotal)).toContain("25%");
    expect(progressText(withTotal)).toContain("250 B");
    expect(progressText(withTotal)).toContain("1000 B");
    const noTotal: ProgressView = toProgressView({ downloaded: 4096, total: null });
    expect(progressText(noTotal)).toContain("4.0 KB");
    expect(progressText(noTotal)).not.toContain("%");
  });

  it("shouldRefreshProgress：首次必刷；百分比变化 ≥1 立即刷；未到间隔且百分比没动不刷；到间隔必刷", () => {
    const p = (percent: number | null): ProgressView =>
      percent === null
        ? toProgressView({ downloaded: 100, total: null })
        : toProgressView({ downloaded: percent * 10, total: 1000 });

    // 首次：必刷
    expect(shouldRefreshProgress(null, p(0), 1000)).toBe(true);

    const shown = { atMs: 1000, percent: 20 };
    // 同一毫秒内百分比没动：不刷（逐 chunk 事件被节流掉）
    expect(shouldRefreshProgress(shown, p(20), 1050)).toBe(false);
    // 百分比跳了 ≥1：立即刷（不等 200ms）
    expect(shouldRefreshProgress(shown, p(21), 1010)).toBe(true);
    // 不足 1 个百分点：不刷
    expect(shouldRefreshProgress(shown, toProgressView({ downloaded: 209, total: 1000 }), 1050)).toBe(false);
    // 到了间隔（默认 200ms）：即使百分比没动也刷
    expect(shouldRefreshProgress(shown, p(20), 1200)).toBe(true);
    // 无 total（百分比 null）：只按时间节流
    expect(shouldRefreshProgress(shown, p(null), 1050)).toBe(false);
    expect(shouldRefreshProgress(shown, p(null), 1201)).toBe(true);
    // 自定义间隔生效
    expect(shouldRefreshProgress(shown, p(20), 1400, 500)).toBe(false);
    expect(shouldRefreshProgress(shown, p(20), 1500, 500)).toBe(true);
  });
});
