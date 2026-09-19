import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import WebUiPanel from "./WebUiPanel.vue";
import type { NetworkSegment, WebUiConfigInfo, WebUiSaveOutcome } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（命令包装按 cmdName 透传给唯一的 invokeMock）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});

const segments: NetworkSegment[] = [
  { cidr: "100.84.0.0/16", encrypted_mesh: true, label: "NetBird 虚拟网" },
  { cidr: "192.168.1.0/24", encrypted_mesh: false, label: null },
];

function configFixture(overrides: Partial<WebUiConfigInfo> = {}): WebUiConfigInfo {
  return {
    enabled: false,
    segments: [],
    port: 17321,
    token: "0123456789abcdef0123456789abcdef",
    token_generated_at: "2026-09-19 08:00:00",
    ...overrides,
  };
}

function outcomeFixture(overrides: Partial<WebUiSaveOutcome> = {}): WebUiSaveOutcome {
  return {
    config: configFixture(),
    firewall_ok: true,
    firewall_error: null,
    firewall_manual_cmd: null,
    server_ok: true,
    server_error: null,
    ...overrides,
  };
}

interface Extra {
  config?: WebUiConfigInfo;
  segments?: NetworkSegment[];
  saveOutcome?: WebUiSaveOutcome;
  saveError?: string;
}

async function openPanel(extra: Extra = {}) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "get_webui_config":
        return extra.config ?? configFixture();
      case "list_network_segments":
        return extra.segments ?? segments;
      case "save_webui_config":
        if (extra.saveError) throw extra.saveError;
        return extra.saveOutcome ?? outcomeFixture();
      case "regenerate_token":
        return configFixture({ token: "ffffffffffffffffffffffffffffffff" });
      case "get_access_url":
        return "http://100.84.12.3:17321/#token=0123456789abcdef0123456789abcdef";
      default:
        return null;
    }
  });
  const wrapper = mount(WebUiPanel);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  document.body.innerHTML = "";
});

describe("设置「网页端」面板（webui-checkin 票 03）", () => {
  it("加载配置与网段：总开关默认关、端口回显、NetBird 段置顶带名", async () => {
    const wrapper = await openPanel();

    expect((wrapper.find(".webui-enabled-input").element as HTMLInputElement).checked).toBe(false);
    expect((wrapper.find(".webui-port-input").element as HTMLInputElement).value).toBe("17321");
    const rows = wrapper.findAll(".seg-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("NetBird 虚拟网");
    // 凭证打码显示（凭证区不露明文；完整地址区按设计明示含凭证的完整地址）
    expect(wrapper.find(".token-masked").text()).toBe("0123…cdef");
    expect(wrapper.find(".token-plain").exists()).toBe(false);
  });

  it("保存：合法入参按三项发出 save_webui_config，成功回显已保存并抛 changed", async () => {
    const wrapper = await openPanel();

    await wrapper.find(".webui-enabled-input").setValue(true);
    // 勾 NetBird 段（加密身份，即时勾上）
    await wrapper.findAll(".seg-check")[0].trigger("change");
    invokeMock.mockClear();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "save_webui_config"
        ? outcomeFixture({
            config: configFixture({ enabled: true, segments: ["100.84.0.0/16"] }),
          })
        : null,
    );
    await wrapper.find(".save-webui-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_webui_config", {
      input: { enabled: true, segments: ["100.84.0.0/16"], port: 17321 },
    });
    expect(wrapper.find(".webui-saved").text()).toContain("已保存");
    expect(wrapper.emitted("changed")).toBeTruthy();
  });

  it("端口非法（0/70000/空）→ 前端拦截报错，不发 save_webui_config（验收：端口校验）", async () => {
    for (const bad of ["0", "70000", ""]) {
      const wrapper = await openPanel();
      invokeMock.mockClear();
      await wrapper.find(".webui-port-input").setValue(bad);
      await wrapper.find(".save-webui-btn").trigger("click");
      await flushPromises();

      expect(wrapper.find(".webui-error").text()).toContain("1024–65535");
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_webui_config")).toBe(false);
    }
  });

  it("防火墙同步失败 → 配置已保存回显 + 错误与现成 netsh 手动命令展示（验收 5）", async () => {
    const wrapper = await openPanel({
      saveOutcome: outcomeFixture({
        config: configFixture({ enabled: true, segments: ["100.84.0.0/16"] }),
        firewall_ok: false,
        firewall_error: "防火墙规则同步失败（退出码 Some(1)）：UAC 被拒绝",
        firewall_manual_cmd:
          'netsh advfirewall firewall delete rule name="AntFeedingLog WebUI"\nnetsh advfirewall firewall add rule name="AntFeedingLog WebUI" dir=in action=allow protocol=TCP localport=17321 remoteip=100.84.0.0/16',
      }),
    });

    await wrapper.find(".save-webui-btn").trigger("click");
    await flushPromises();

    // 半成功语义：配置保存成功照常回显
    expect(wrapper.find(".webui-saved").text()).toContain("已保存");
    expect(wrapper.find(".firewall-fail-msg").text()).toContain("UAC 被拒绝");
    expect(wrapper.find(".firewall-manual").text()).toContain("netsh advfirewall firewall add rule");
    expect(wrapper.find(".firewall-manual").text()).toContain("AntFeedingLog WebUI");
    expect(wrapper.find(".copy-manual-btn").exists()).toBe(true);
  });

  it("防火墙成功 → 不出失败区", async () => {
    const wrapper = await openPanel();
    await wrapper.find(".save-webui-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".firewall-fail").exists()).toBe(false);
  });

  it("端口被占用 → 配置已保存照常回显 + 服务人话错误展示（票 04）", async () => {
    const wrapper = await openPanel({
      saveOutcome: outcomeFixture({
        config: configFixture({ enabled: true }),
        server_ok: false,
        server_error: "端口 17321 被占用或不可用（拒绝访问），网页端服务未启动；可改用其他端口后重新保存",
      }),
    });

    await wrapper.find(".save-webui-btn").trigger("click");
    await flushPromises();

    // 半成功语义：配置保存成功照常回显，服务错误单独展示
    expect(wrapper.find(".webui-saved").text()).toContain("已保存");
    expect(wrapper.find(".server-fail-msg").text()).toContain("端口 17321 被占用");
    expect(wrapper.find(".firewall-fail").exists()).toBe(false);
  });

  it("服务正常起停 → 不出服务错误行", async () => {
    const wrapper = await openPanel();
    await wrapper.find(".save-webui-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".server-fail-msg").exists()).toBe(false);
  });

  it("重生成按钮 → regenerate_token，凭证区刷新为新值（验收 2）", async () => {
    const wrapper = await openPanel();

    await wrapper.find(".token-regen").trigger("click");
    await wrapper.find(".regen-ok").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("regenerate_token");
    expect(wrapper.find(".webui-saved").text()).toContain("旧地址即刻作废");
  });
});
