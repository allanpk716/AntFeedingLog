import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import WebUiAccessUrl from "./WebUiAccessUrl.vue";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的 invokeMock）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

const URL = "http://100.84.12.3:17321/#token=0123456789abcdef0123456789abcdef";

function mountBox(refreshKey = 0) {
  return mount(WebUiAccessUrl, { props: { refreshKey } });
}

beforeEach(() => {
  invokeMock.mockReset();
  document.body.innerHTML = "";
});

describe("完整访问地址区（webui-checkin 票 03）", () => {
  it("挂载即拉 get_access_url，展示含 #token= 的完整地址（验收 3）", async () => {
    invokeMock.mockResolvedValue(URL);
    const wrapper = mountBox();
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("get_access_url");
    expect(wrapper.find(".access-url").text()).toBe(URL);
    expect(wrapper.find(".access-url").text()).toContain("#token=");
    expect(wrapper.find(".access-url-error").exists()).toBe(false);
  });

  it("拉取失败（网段不在线等）→ 展示原因，不出地址与复制按钮", async () => {
    invokeMock.mockRejectedValue("所选网段 100.84.0.0/16 上未发现本机地址（网段当前不在线？…）");
    const wrapper = mountBox();
    await flushPromises();

    expect(wrapper.find(".access-url-error").text()).toContain("未发现本机地址");
    expect(wrapper.find(".access-url").exists()).toBe(false);
    expect(wrapper.find(".copy-url-btn").exists()).toBe(false);
  });

  it("一键复制走 Clipboard API，复制的是完整含凭证地址（验收 3）", async () => {
    invokeMock.mockResolvedValue(URL);
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const wrapper = mountBox();
    await flushPromises();

    await wrapper.find(".copy-url-btn").trigger("click");
    await flushPromises();

    expect(writeText).toHaveBeenCalledWith(URL);
    expect(wrapper.find(".copy-ok").text()).toContain("已复制");
  });

  it("复制失败 → 提示手动复制，不误报已复制", async () => {
    invokeMock.mockResolvedValue(URL);
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: vi.fn(async () => {
          throw new Error("denied");
        }),
      },
      configurable: true,
    });
    // 降级 execCommand 也失败
    const original = document.execCommand;
    Object.defineProperty(document, "execCommand", {
      value: () => false,
      configurable: true,
      writable: true,
    });
    const wrapper = mountBox();
    await flushPromises();
    try {
      await wrapper.find(".copy-url-btn").trigger("click");
      await flushPromises();
      expect(wrapper.find(".copy-fail").text()).toContain("复制失败");
      expect(wrapper.find(".copy-ok").exists()).toBe(false);
    } finally {
      Object.defineProperty(document, "execCommand", {
        value: original,
        configurable: true,
        writable: true,
      });
    }
  });

  it("refreshKey 变化（保存/重生成后）重新拉取", async () => {
    invokeMock.mockResolvedValue("http://old:17321/#token=a");
    const wrapper = mountBox(0);
    await flushPromises();
    expect(wrapper.find(".access-url").text()).toContain("old");

    invokeMock.mockClear();
    invokeMock.mockResolvedValue("http://new:17321/#token=b");
    await wrapper.setProps({ refreshKey: 1 });
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("get_access_url");
    expect(wrapper.find(".access-url").text()).toContain("new");
  });

  it("固定风险提示常驻（勿转发/可重生成作废）", async () => {
    invokeMock.mockResolvedValue(URL);
    const wrapper = mountBox();
    await flushPromises();

    expect(wrapper.find(".risk-hint").text()).toContain("勿转发");
    expect(wrapper.find(".risk-hint").text()).toContain("重生成");
  });
});
