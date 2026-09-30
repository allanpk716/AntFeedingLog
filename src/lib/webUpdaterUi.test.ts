import { describe, expect, it } from "vitest";
import {
  RESTART_WATCH_MS,
  SERVER_PROBE_INTERVAL_MS,
  WEB_BANNER_ACTION_TEXT,
  WEB_CONFIRM_TEXT,
  bannerForDetail,
  confirmViewFor,
  isNoNewerVersionError,
  isRestartWatchExpired,
  restartingText,
  restartTimeoutText,
  shouldReloadForEpoch,
  upgradeResultForReload,
  waitingRestartText,
  webBannerText,
} from "./webUpdaterUi";
import { installFailedText, installInProgressText } from "./updaterUi";

describe("确认结果 → 展示视图（confirmViewFor）", () => {
  it("install_started → waiting-restart（带目标版本）", () => {
    expect(confirmViewFor({ status: "install_started", version: "0.3.0" })).toEqual({
      kind: "waiting-restart",
      version: "0.3.0",
    });
  });

  it("install_failed → failed 红条（文案走 installFailedText 口径，重试可用）", () => {
    expect(confirmViewFor({ status: "install_failed", version: "0.3.0", message: "下载更新失败: 请求超时" })).toEqual({
      kind: "failed",
      text: installFailedText("0.3.0", "下载更新失败: 请求超时"),
      retryable: true,
    });
  });

  it("Err 防重入固定串 → 平静「进行中」提示（复用 updaterUi 口径，不走失败引导）", () => {
    expect(confirmViewFor("已有安装流程正在进行，请稍候")).toEqual({
      kind: "install-in-progress",
      text: installInProgressText(),
    });
  });

  it("Err 复查无新版固定串 → no-newer（红条不弹重试，收横幅重读红点）", () => {
    expect(confirmViewFor("远端已没有比当前更新的版本")).toEqual({
      kind: "no-newer",
      text: "远端已没有比当前更新的版本",
    });
  });

  it("Err 其余检查失败 → failed 红条带原因（版本用横幅目标兜底），重试可用", () => {
    expect(confirmViewFor("检查更新失败: HTTP 404", "0.3.0")).toEqual({
      kind: "failed",
      text: installFailedText("0.3.0", "检查更新失败: HTTP 404"),
      retryable: true,
    });
  });
});

describe("复查无新版识别（isNoNewerVersionError）", () => {
  it("固定串原样与夹杂场景命中，其余错误不误判", () => {
    expect(isNoNewerVersionError("远端已没有比当前更新的版本")).toBe(true);
    expect(isNoNewerVersionError("升级失败: 远端已没有比当前更新的版本")).toBe(true);
    expect(isNoNewerVersionError("检查更新失败: HTTP 404")).toBe(false);
    expect(isNoNewerVersionError("已有安装流程正在进行，请稍候")).toBe(false);
  });
});

describe("红点详情 → 横幅数据（bannerForDetail）", () => {
  it("available=true 且版本非空才出横幅，notes 原样透传", () => {
    expect(bannerForDetail({ available: true, version: "0.3.0", notes: null })).toEqual({
      version: "0.3.0",
      notes: null,
    });
    expect(bannerForDetail({ available: true, version: "0.3.0", notes: "修复若干问题" })).toEqual({
      version: "0.3.0",
      notes: "修复若干问题",
    });
  });

  it("无新版 / 版本缺失 / 响应为 null 全按不出横幅（宁可漏报不误报）", () => {
    expect(bannerForDetail({ available: false, version: null, notes: null })).toBeNull();
    expect(bannerForDetail({ available: true, version: null, notes: null })).toBeNull();
    expect(bannerForDetail({ available: true, version: "", notes: null })).toBeNull();
    expect(bannerForDetail(null)).toBeNull();
    expect(bannerForDetail(undefined)).toBeNull();
  });
});

describe("重启等待兜底（出口2：确认起 10 分钟）", () => {
  it("窗口常量 = 10 分钟（规格 F7 取值），探活间隔 3 秒", () => {
    expect(RESTART_WATCH_MS).toBe(10 * 60 * 1000);
    expect(SERVER_PROBE_INTERVAL_MS).toBe(3_000);
  });

  it("isRestartWatchExpired：不足 10 分钟 false，到点即 true（边界含）", () => {
    const t0 = 1_000_000;
    expect(isRestartWatchExpired(t0, t0 + RESTART_WATCH_MS - 1)).toBe(false);
    expect(isRestartWatchExpired(t0, t0 + RESTART_WATCH_MS)).toBe(true);
    expect(isRestartWatchExpired(t0, t0 + RESTART_WATCH_MS + 60_000)).toBe(true);
  });
});

describe("SSE 新 epoch → 该不该整页刷新（出口4 的触发半边）", () => {
  it("epoch 与确认时刻基线不同 = 服务端进程重启过 → 刷新；同 epoch（写帧/网络闪断重连）不刷", () => {
    expect(shouldReloadForEpoch("e1", "e2")).toBe(true);
    expect(shouldReloadForEpoch("e1", "e1")).toBe(false);
  });

  it("基线缺失（没见过任何帧，无从判定）不刷，交给 10 分钟兜底", () => {
    expect(shouldReloadForEpoch(null, "e2")).toBe(false);
  });
});

describe("刷新落地自证（出口4 的比对半边：upgradeResultForReload）", () => {
  it("无记录 none；目标版本与 bundle 相等 upgraded；旧版本回来 stale", () => {
    expect(upgradeResultForReload(null, "0.7.11")).toBe("none");
    expect(upgradeResultForReload("0.7.11", "0.7.11")).toBe("upgraded");
    expect(upgradeResultForReload("0.8.0", "0.7.11")).toBe("stale");
  });
});

describe("文案（中文文案集中在本层，组件只渲染）", () => {
  it("横幅文案带目标版本，动作/确认按钮文案钉死", () => {
    expect(webBannerText("0.3.0")).toBe("有新版本 v0.3.0");
    expect(WEB_BANNER_ACTION_TEXT).toBe("点击升级");
    expect(WEB_CONFIRM_TEXT).toBe("确认升级");
  });

  it("等待/重启/兜底三段文案与规格 D 一致", () => {
    expect(waitingRestartText()).toBe("正在下载升级包，应用将自动重启，本页会短暂断开——恢复后自动刷新");
    expect(restartingText()).toBe("正在重启中…");
    expect(restartTimeoutText()).toBe("升级可能未完成，请到电脑端确认，或稍后刷新本页");
  });
});
