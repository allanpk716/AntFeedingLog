import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import pkg from "../../package.json";
import WebUpdateBanner from "./WebUpdateBanner.vue";
import { clearToasts, toastItems } from "../lib/toast";
import { PENDING_UPGRADE_KEY, restartingText, restartTimeoutText, waitingRestartText } from "../lib/webUpdaterUi";
import { succeededText } from "../lib/updaterUi";

// 不依赖 Tauri 运行时：统一 mock 调用层；subscribe 注入可控桩（按事件名记
// 处理器，emitDataVersion 派发 SSE 帧）；isTauri 经旗标切换桌面/浏览器形态
//（默认浏览器，与 happy-dom 环境一致；桌面形态在专属用例单独打开）。
const { invokeMock, subscribeMock, emitDataVersion, handlerCount, tauriFlag } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  const handlers = new Set<(payload: unknown) => void>();
  const subscribeMock = vi.fn(async (_event: string, handler: (payload: unknown) => void) => {
    handlers.add(handler);
    // 退订必须真移除处理器：组件带状态（waiting 相位会触发 reload），
    // 泄漏到后续用例会串台
    return () => {
      handlers.delete(handler);
    };
  });
  return {
    invokeMock,
    subscribeMock,
    tauriFlag: { value: false },
    handlerCount: () => handlers.size,
    emitDataVersion: (frame: unknown) => {
      for (const handler of [...handlers]) handler(frame);
    },
  };
});
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock, {
    subscribe: subscribeMock,
    isTauri: () => tauriFlag.value,
  })(importOriginal);
});

function badgeDetail(available: boolean, version: string | null = "0.3.0", notes: string | null = null) {
  return { available, version, notes };
}

function mockDefault(detail: unknown): void {
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === "get_update_badge_detail" ? detail : null,
  );
}

async function mountBanner(detail?: unknown) {
  if (detail !== undefined) mockDefault(detail);
  const wrapper = mount(WebUpdateBanner);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  subscribeMock.mockClear();
  tauriFlag.value = false;
  sessionStorage.clear();
  clearToasts();
});

describe("挂载与横幅显隐", () => {
  it("桌面模式（isTauri=true）：不渲染横幅、不发红点查询、不订阅", async () => {
    tauriFlag.value = true;
    const wrapper = await mountBanner(badgeDetail(true));

    expect(invokeMock).not.toHaveBeenCalled();
    expect(subscribeMock).not.toHaveBeenCalled();
    expect(wrapper.find(".web-update").exists()).toBe(false);
  });

  it("首查未回来不渲染横幅（数据未到不闪）；available=true 到达后出横幅「有新版本 vY → 点击升级」", async () => {
    let resolveBadge!: (v: unknown) => void;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_update_badge_detail")
        return new Promise((resolve) => {
          resolveBadge = resolve;
        });
      return null;
    });
    const wrapper = mount(WebUpdateBanner);
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("get_update_badge_detail");
    expect(wrapper.find(".web-update").exists()).toBe(false);

    resolveBadge(badgeDetail(true, "0.3.0"));
    await flushPromises();

    const banner = wrapper.find(".wu-banner");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("有新版本 v0.3.0");
    expect(banner.find(".wu-open-btn").text()).toBe("点击升级");
    expect(subscribeMock).toHaveBeenCalledWith("data-version", expect.any(Function));
  });

  it("无新版（available=false）与首查失败都静默不出横幅（漏报不误报）", async () => {
    const wrapper = await mountBanner(badgeDetail(false, null, null));
    expect(wrapper.find(".web-update").exists()).toBe(false);

    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === "get_update_badge_detail" ? Promise.reject("HTTP 500") : null,
      );
      const w2 = mount(WebUpdateBanner);
      await flushPromises();
      expect(w2.find(".web-update").exists()).toBe(false);
    } finally {
      errorSpy.mockRestore();
    }
  });

  it("× 关闭后本次会话不再出现：重查（红点再读）也不复活（组件内会话态，不落持久存储）", async () => {
    const wrapper = await mountBanner(badgeDetail(true));
    expect(wrapper.find(".wu-banner").exists()).toBe(true);

    await wrapper.find(".wu-close-btn").trigger("click");
    expect(wrapper.find(".web-update").exists()).toBe(false);

    // 会话内重查：即使远端仍报有新版也不复活
    await (wrapper.vm as unknown as { recheckBadge: () => Promise<void> }).recheckBadge();
    await flushPromises();
    expect(wrapper.find(".wu-banner").exists()).toBe(false);
    expect(sessionStorage.getItem(PENDING_UPGRADE_KEY)).toBeNull();
  });
});

describe("确认面板", () => {
  it("点「点击升级」开面板：当前 vX（package.json）→ 新版 vY + 更新说明 + 确认升级", async () => {
    const wrapper = await mountBanner(badgeDetail(true, "0.3.0", "修复若干问题"));
    await wrapper.find(".wu-open-btn").trigger("click");

    const panel = wrapper.find(".wu-panel");
    expect(panel.exists()).toBe(true);
    expect(panel.text()).toContain(`当前 v${pkg.version}`);
    expect(panel.text()).toContain("新版本 v0.3.0");
    expect(panel.find(".wu-notes").text()).toContain("修复若干问题");
    expect(panel.find(".wu-confirm-btn").text()).toBe("确认升级");
  });

  it("notes=null 省略说明区（F10）", async () => {
    const wrapper = await mountBanner(badgeDetail(true, "0.4.0", null));
    await wrapper.find(".wu-open-btn").trigger("click");

    expect(wrapper.find(".wu-panel").exists()).toBe(true);
    expect(wrapper.find(".wu-panel .wu-notes").exists()).toBe(false);
  });

  it("取消收面板回横幅，不发确认命令", async () => {
    const wrapper = await mountBanner(badgeDetail(true));
    await wrapper.find(".wu-open-btn").trigger("click");
    await wrapper.find(".wu-cancel-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".wu-panel").exists()).toBe(false);
    expect(wrapper.find(".wu-banner").exists()).toBe(true);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "confirm_and_install_update")).toBe(false);
  });
});

describe("升级状态机（规格 D 出口 1/3 + 防重入）", () => {
  async function mountToConfirm() {
    const wrapper = await mountBanner(badgeDetail(true, "0.3.0", null));
    await wrapper.find(".wu-open-btn").trigger("click");
    return wrapper;
  }

  it("出口1：install_failed 响应（HTTP 生命周期内回来）→ 红条带原因 + 重试；重试重发确认", async () => {
    let resolveConfirm!: (v: unknown) => void;
    const wrapper = await mountToConfirm();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install_update")
        return new Promise((resolve) => {
          resolveConfirm = resolve;
        });
      return null;
    });

    await wrapper.find(".wu-confirm-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("confirm_and_install_update");
    // busy：确认中，按钮禁用
    expect((wrapper.find(".wu-confirm-btn").element as HTMLButtonElement).disabled).toBe(true);
    expect(wrapper.find(".wu-confirm-btn").text()).toContain("确认中");

    resolveConfirm({ status: "install_failed", version: "0.3.0", message: "下载更新失败: 请求超时" });
    await flushPromises();

    const failed = wrapper.find(".wu-failed");
    expect(failed.exists()).toBe(true);
    expect(failed.text()).toContain("v0.3.0");
    expect(failed.text()).toContain("下载更新失败: 请求超时");
    expect(wrapper.find(".wu-retry-btn").exists()).toBe(true);
    // 规格 D 出口1：红色轻提示带原因（全局 toast 模块）
    const errorToast = toastItems.value.find((t) => t.kind === "error");
    expect(errorToast?.message).toContain("下载更新失败: 请求超时");

    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "confirm_and_install_update" ? { status: "install_started", version: "0.3.0" } : null,
    );
    await wrapper.find(".wu-retry-btn").trigger("click");
    await flushPromises();

    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "confirm_and_install_update").length,
    ).toBe(2);
    expect(wrapper.find(".wu-waiting").exists()).toBe(true);
    // 收尾卸载：waiting 相位的组件不把帧处理器泄漏进后续用例
    wrapper.unmount();
  });

  it("出口3：Err「远端已没有比当前更新的版本」→ 红条带该文案、不弹重试；收横幅并重读红点（重读仍报有新版也不复活）", async () => {
    const wrapper = await mountToConfirm();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install_update") throw "远端已没有比当前更新的版本";
      if (cmd === "get_update_badge_detail") return badgeDetail(true, "0.3.0", null);
      return null;
    });

    await wrapper.find(".wu-confirm-btn").trigger("click");
    await flushPromises();

    const failed = wrapper.find(".wu-failed");
    expect(failed.exists()).toBe(true);
    expect(failed.text()).toContain("远端已没有比当前更新的版本");
    expect(wrapper.find(".wu-retry-btn").exists()).toBe(false);
    // 规格 D 出口3：红色轻提示带该原因
    const errorToast = toastItems.value.find((t) => t.kind === "error");
    expect(errorToast?.message).toContain("远端已没有比当前更新的版本");
    // 收横幅 + 重读红点：详情命令被再次调用，且横幅不复活
    expect(wrapper.find(".wu-banner").exists()).toBe(false);
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "get_update_badge_detail").length,
    ).toBeGreaterThanOrEqual(2);
    wrapper.unmount();
  });

  it("防重入 Err（复用 isInstallInProgressError 口径）→ 平静提示，不出重试、不出失败引导", async () => {
    const wrapper = await mountToConfirm();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "confirm_and_install_update" ? Promise.reject("已有安装流程正在进行，请稍候") : null,
    );

    await wrapper.find(".wu-confirm-btn").trigger("click");
    await flushPromises();

    const calm = wrapper.find(".wu-calm");
    expect(calm.exists()).toBe(true);
    expect(calm.text()).toContain("正在进行");
    expect(wrapper.find(".wu-retry-btn").exists()).toBe(false);
    expect(wrapper.find(".wu-failed").exists()).toBe(false);
    wrapper.unmount();
  });
});

describe("升级等待与重启（规格 D 出口 2/4）", () => {
  it("install_started → 等待文案 + sessionStorage 记目标版本；同 epoch 帧不刷、新 epoch 帧整页刷新（出口4 触发）", async () => {
    const reloadSpy = vi.spyOn(window.location, "reload").mockImplementation(() => {});
    const wrapper = await mountBanner(badgeDetail(true, "0.3.0", null));
    emitDataVersion({ type: "hello", epoch: "e1", version: 1 }); // 确认前的基线 epoch

    await wrapper.find(".wu-open-btn").trigger("click");
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install_update") return { status: "install_started", version: "0.3.0" };
      if (cmd === "health_check") return { schema_version: 14 };
      return null;
    });
    await wrapper.find(".wu-confirm-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".wu-waiting").text()).toBe(waitingRestartText());
    expect(sessionStorage.getItem(PENDING_UPGRADE_KEY)).toBe("0.3.0");

    // 同 epoch（服务端还没重启，如快照写/网络闪断重连）：不刷
    emitDataVersion({ type: "hello", epoch: "e1", version: 2 });
    expect(reloadSpy).not.toHaveBeenCalled();

    // 新 epoch = 服务端进程重启过 → 整页刷新拿新 bundle
    emitDataVersion({ type: "hello", epoch: "e2", version: 3 });
    expect(reloadSpy).toHaveBeenCalledTimes(1);

    wrapper.unmount();
    reloadSpy.mockRestore();
  });

  it("等待期探活失败（服务端已退出）→ 切「正在重启中…」", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const wrapper = await mountBanner(badgeDetail(true, "0.3.0", null));
      await wrapper.find(".wu-open-btn").trigger("click");
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === "confirm_and_install_update"
          ? { status: "install_started", version: "0.3.0" }
          : cmd === "health_check"
            ? Promise.reject("HTTP 500")
            : null,
      );
      await wrapper.find(".wu-confirm-btn").trigger("click");
      await flushPromises();
      expect(wrapper.find(".wu-waiting").text()).toBe(waitingRestartText());

      await vi.advanceTimersByTimeAsync(3_000);
      expect(wrapper.find(".wu-waiting").text()).toBe(restartingText());

      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("出口2：确认起 10 分钟没等到重启 → restart-timeout 文案、无重试按钮；探活停止、此后新 epoch 也不再刷新", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
    try {
      vi.setSystemTime(0);
      const reloadSpy = vi.spyOn(window.location, "reload").mockImplementation(() => {});
      const wrapper = await mountBanner(badgeDetail(true, "0.3.0", null));
      await wrapper.find(".wu-open-btn").trigger("click");
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "confirm_and_install_update") return { status: "install_started", version: "0.3.0" };
        if (cmd === "health_check") return { schema_version: 14 };
        return null;
      });
      await wrapper.find(".wu-confirm-btn").trigger("click");
      await flushPromises();

      await vi.advanceTimersByTimeAsync(10 * 60 * 1000);

      const timeout = wrapper.find(".wu-timeout");
      expect(timeout.exists()).toBe(true);
      expect(timeout.text()).toBe(restartTimeoutText());
      expect(wrapper.find(".wu-retry-btn").exists()).toBe(false);

      // 兜底收尾后探活停摆
      const probes = invokeMock.mock.calls.filter(([cmd]) => cmd === "health_check").length;
      await vi.advanceTimersByTimeAsync(9_000);
      expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "health_check").length).toBe(probes);

      // 此后即使新 epoch 帧到达也不再刷新（已转手动刷新出口）
      emitDataVersion({ type: "hello", epoch: "e2", version: 3 });
      expect(reloadSpy).not.toHaveBeenCalled();

      wrapper.unmount();
      reloadSpy.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });

  it("卸载清理：等待期卸载后探活停止、SSE 订阅退净", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
    try {
      const subscribed = handlerCount();
      const wrapper = await mountBanner(badgeDetail(true, "0.3.0", null));
      await wrapper.find(".wu-open-btn").trigger("click");
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === "confirm_and_install_update"
          ? { status: "install_started", version: "0.3.0" }
          : cmd === "health_check"
            ? { schema_version: 14 }
            : null,
      );
      await wrapper.find(".wu-confirm-btn").trigger("click");
      await flushPromises();
      expect(handlerCount()).toBe(subscribed + 1);
      const probes = invokeMock.mock.calls.filter(([cmd]) => cmd === "health_check").length;

      wrapper.unmount();
      await vi.advanceTimersByTimeAsync(9_000);

      expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "health_check").length).toBe(probes);
      expect(handlerCount()).toBe(subscribed);
    } finally {
      vi.useRealTimers();
    }
  });

  it("出口4 比对：刷新落地后 sessionStorage 目标版本 = bundle 版本 → 弹「已升级到 vX」并清记录", async () => {
    sessionStorage.setItem(PENDING_UPGRADE_KEY, pkg.version);
    const wrapper = await mountBanner(badgeDetail(false, null, null));

    expect(toastItems.value.map((t) => t.message)).toEqual([succeededText(pkg.version)]);
    expect(toastItems.value[0]?.kind).toBe("success");
    expect(sessionStorage.getItem(PENDING_UPGRADE_KEY)).toBeNull();
    expect(wrapper.find(".wu-banner").exists()).toBe(false);
  });

  it("出口4 比对：旧版本回来了（记录与 bundle 不等）→ 不弹、仍清记录", async () => {
    sessionStorage.setItem(PENDING_UPGRADE_KEY, "0.0.1");
    await mountBanner(badgeDetail(false, null, null));

    expect(toastItems.value.length).toBe(0);
    expect(sessionStorage.getItem(PENDING_UPGRADE_KEY)).toBeNull();
  });
});
