import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import UpdatePanel from "./UpdatePanel.vue";

// 不依赖 Tauri 运行时：mock 掉 IPC 与事件监听（沿 LogListPage.test.ts 先例）
const { invokeMock, listenMock, unlistenMock, emitProgress } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  const unlistenMock = vi.fn();
  let handler: ((e: { payload: unknown }) => void) | null = null;
  const listenMock = vi.fn(async (_event: string, h: (e: { payload: unknown }) => void) => {
    handler = h;
    return unlistenMock;
  });
  return {
    invokeMock,
    listenMock,
    unlistenMock,
    emitProgress: (payload: unknown) => handler?.({ payload }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

function baseMock(updateState: object = { status: "idle" }) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "get_app_version":
        return "0.1.0";
      case "get_update_state":
        return updateState;
      default:
        return null;
    }
  });
}

async function mountPanel(updateState?: object) {
  baseMock(updateState);
  const wrapper = mount(UpdatePanel);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockClear();
  unlistenMock.mockClear();
});

describe("更新面板（票 06）：挂载与启动残留引导", () => {
  it("挂载取当前版本与更新状态；idle 不出横幅，并已订阅下载进度事件", async () => {
    const wrapper = await mountPanel();

    expect(invokeMock).toHaveBeenCalledWith("get_app_version");
    expect(invokeMock).toHaveBeenCalledWith("get_update_state");
    expect(wrapper.find(".current-version").text()).toContain("v0.1.0");
    expect(wrapper.find(".update-banner-warn").exists()).toBe(false);
    expect(wrapper.find(".update-banner-ok").exists()).toBe(false);
    expect(listenMock).toHaveBeenCalledWith("update-download-progress", expect.any(Function));
  });

  it("update_state 为失败残留 → 顶部出「上次升级未完成」引导（含目标版本与手动下载出口）", async () => {
    const wrapper = await mountPanel({ status: "last_install_incomplete", version: "0.3.0" });

    const banner = wrapper.find(".update-banner-warn");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("上次升级未完成");
    expect(banner.text()).toContain("v0.3.0");
    expect(banner.text()).toContain("手动下载");

    invokeMock.mockClear();
    await banner.find(".manual-dl-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("open_releases_page");
  });

  it("update_state 为升级成功 → 平静的「已升级到 vX」提示", async () => {
    const wrapper = await mountPanel({ status: "last_install_succeeded", version: "0.2.0" });

    const banner = wrapper.find(".update-banner-ok");
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("已升级到 v0.2.0");
  });

  it("卸载时退订进度事件", async () => {
    const wrapper = await mountPanel();
    wrapper.unmount();
    expect(unlistenMock).toHaveBeenCalledTimes(1);
  });

  it("挂载后 listen 未 resolve 就卸载：resolve 后立即退订，不泄漏监听（评审 R2 Minor-1）", async () => {
    let resolveListen!: (fn: typeof unlistenMock) => void;
    listenMock.mockImplementationOnce(
      () =>
        new Promise<typeof unlistenMock>((resolve) => {
          resolveListen = resolve;
        }),
    );
    baseMock();
    const wrapper = mount(UpdatePanel);
    wrapper.unmount(); // listen 还没 resolve 就卸载
    resolveListen(unlistenMock);
    await flushPromises();
    expect(unlistenMock).toHaveBeenCalledTimes(1);
  });
});

describe("更新面板：立即检查更新三态", () => {
  it("无更新 → 平静的「已是最新」", async () => {
    const wrapper = await mountPanel();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "check_update_now" ? { status: "up_to_date" } : null,
    );

    await wrapper.find(".check-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("check_update_now");
    expect(wrapper.find(".update-ok").text()).toContain("已是最新");
  });

  it("有新版 → 版本号 + 说明 + 确认操作（下载并安装 / 暂不更新）", async () => {
    const wrapper = await mountPanel();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "check_update_now"
        ? { status: "update_available", version: "0.3.0", notes: "修复若干问题" }
        : null,
    );

    await wrapper.find(".check-btn").trigger("click");
    await flushPromises();

    const title = wrapper.find(".update-title");
    expect(title.text()).toContain("v0.3.0");
    expect(wrapper.find(".update-notes").text()).toContain("修复若干问题");
    expect(wrapper.find(".install-btn").exists()).toBe(true);
    expect(wrapper.find(".decline-btn").exists()).toBe(true);
  });

  it("有新版但说明为空 → 不渲染说明行", async () => {
    const wrapper = await mountPanel();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "check_update_now"
        ? { status: "update_available", version: "0.3.0", notes: null }
        : null,
    );

    await wrapper.find(".check-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".update-title").text()).toContain("v0.3.0");
    expect(wrapper.find(".update-notes").exists()).toBe(false);
  });

  it("检查失败 → 一次性错误提示（检查入口仍可用）", async () => {
    const wrapper = await mountPanel();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_update_now") throw "检查更新失败: HTTP 404";
      return null;
    });

    await wrapper.find(".check-btn").trigger("click");
    await flushPromises();

    const err = wrapper.find(".check-error");
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain("HTTP 404");
    // 不轰炸重试：只有一条错误提示，按钮恢复可点
    expect(wrapper.findAll(".check-error").length).toBe(1);
    expect((wrapper.find(".check-btn").element as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("更新面板：确认安装交互流", () => {
  async function mountWithAvailable() {
    const wrapper = await mountPanel();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_update_now")
        return { status: "update_available", version: "0.3.0", notes: null };
      return null;
    });
    await wrapper.find(".check-btn").trigger("click");
    await flushPromises();
    return wrapper;
  }

  it("取消（暂不更新）不触发安装，确认行收起", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockClear();

    await wrapper.find(".decline-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".install-btn").exists()).toBe(false);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "confirm_and_install")).toBe(false);
  });

  it("确认 → 进度展示（节流）→ install_started 提示将重启以完成安装", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    try {
      const wrapper = await mountWithAvailable();

      let resolveInstall: (v: unknown) => void = () => {};
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "confirm_and_install")
          return new Promise((resolve) => {
            resolveInstall = resolve;
          });
        return null;
      });

      await wrapper.find(".install-btn").trigger("click");
      await flushPromises();

      expect(invokeMock).toHaveBeenCalledWith("confirm_and_install");
      // 确认后进入安装态：确认/取消按钮收起，出现进度区
      expect(wrapper.find(".install-btn").exists()).toBe(false);
      expect(wrapper.find(".progress-text").text()).toContain("准备下载");

      // 逐 chunk 事件：同一时刻百分比没动 → 不上屏（节流掉）
      emitProgress({ downloaded: 0, total: 1000 });
      await flushPromises();
      expect(wrapper.find(".progress-text").text()).toContain("0%");
      emitProgress({ downloaded: 5, total: 1000 });
      await flushPromises();
      expect(wrapper.find(".progress-text").text()).toContain("0 B");
      expect(wrapper.find(".progress-text").text()).not.toContain("1%");

      // 时间推进 200ms+：即使百分比没动也刷
      vi.setSystemTime(250);
      emitProgress({ downloaded: 500, total: 1000 });
      await flushPromises();
      expect(wrapper.find(".progress-text").text()).toContain("50%");

      // 完成：install_started → 「将重启」提示
      resolveInstall({ status: "install_started", version: "0.3.0" });
      await flushPromises();
      const started = wrapper.find(".install-started");
      expect(started.exists()).toBe(true);
      expect(started.text()).toContain("重启");
    } finally {
      vi.useRealTimers();
    }
  });

  it("install_failed → 展示版本 + 原因 + 重试 / 手动下载；重试再次发起确认安装", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install")
        return { status: "install_failed", version: "0.3.0", message: "下载更新失败: 请求超时" };
      return null;
    });

    await wrapper.find(".install-btn").trigger("click");
    await flushPromises();

    const failed = wrapper.find(".install-failed-text");
    expect(failed.exists()).toBe(true);
    expect(failed.text()).toContain("v0.3.0");
    expect(failed.text()).toContain("请求超时");

    invokeMock.mockClear();
    await wrapper.find(".retry-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("confirm_and_install");
  });

  it("确认安装前置失败（Err）→ 也走失败引导，给重试 / 手动下载出口", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install") throw "远端已没有比当前更新的版本";
      return null;
    });

    await wrapper.find(".install-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".install-failed-text").exists()).toBe(true);
    expect(wrapper.find(".retry-btn").exists()).toBe(true);
    expect(wrapper.find(".manual-dl-btn").exists()).toBe(true);
  });

  it("失败引导里的手动下载 → open_releases_page", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install")
        return { status: "install_failed", version: "0.3.0", message: "boom" };
      return null;
    });
    await wrapper.find(".install-btn").trigger("click");
    await flushPromises();

    invokeMock.mockClear();
    await wrapper.find(".install-failed .manual-dl-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("open_releases_page");
  });

  it("在途安装时重复确认（Rust 防重入拒绝）→ 平静「进行中」提示，不出失败引导（评审 R2 Important）", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install") throw "已有安装流程正在进行，请稍候";
      return null;
    });

    await wrapper.find(".install-btn").trigger("click");
    await flushPromises();

    const calm = wrapper.find(".install-in-progress");
    expect(calm.exists()).toBe(true);
    expect(calm.text()).toContain("正在进行");
    // 误报防护锚点：不渲染失败文案与重试/手动下载出口
    expect(wrapper.find(".install-failed-text").exists()).toBe(false);
    expect(wrapper.find(".retry-btn").exists()).toBe(false);
    expect(wrapper.find(".manual-dl-btn").exists()).toBe(false);
  });

  it("install_started 后「立即检查更新」禁用（将重启窗口期，评审 R2 Minor-2）", async () => {
    const wrapper = await mountWithAvailable();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "confirm_and_install")
        return { status: "install_started", version: "0.3.0" };
      return null;
    });

    await wrapper.find(".install-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".install-started").exists()).toBe(true);
    expect((wrapper.find(".check-btn").element as HTMLButtonElement).disabled).toBe(true);
  });
});
