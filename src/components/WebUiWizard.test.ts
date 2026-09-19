import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import WebUiWizard from "./WebUiWizard.vue";
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

const config: WebUiConfigInfo = {
  enabled: false,
  segments: [],
  port: 17321,
  token: "0123456789abcdef0123456789abcdef",
  token_generated_at: "2026-09-19 08:00:00",
};

function outcomeFixture(firewallOk: boolean): WebUiSaveOutcome {
  return {
    config: { ...config, enabled: true, segments: ["100.84.0.0/16"] },
    firewall_ok: firewallOk,
    firewall_error: firewallOk ? null : "防火墙规则同步失败（退出码 Some(1)）：UAC 被拒绝",
    firewall_manual_cmd: firewallOk
      ? null
      : 'netsh advfirewall firewall delete rule name="AntFeedingLog WebUI"',
    server_ok: true,
    server_error: null,
  };
}

interface Extra {
  saveOutcome?: WebUiSaveOutcome;
  saveError?: string;
}

async function openWizard(extra: Extra = {}) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "get_webui_config":
        return config;
      case "list_network_segments":
        return segments;
      case "get_access_url":
        return "http://100.84.12.3:17321/#token=0123456789abcdef0123456789abcdef";
      case "save_webui_config":
        if (extra.saveError) throw extra.saveError;
        return extra.saveOutcome ?? outcomeFixture(true);
      case "mark_webui_wizard_done":
        return null;
      default:
        return null;
    }
  });
  const wrapper = mount(WebUiWizard);
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  invokeMock.mockReset();
  document.body.innerHTML = "";
});

describe("网页端首启向导（webui-checkin 票 03）", () => {
  it("步骤 ①：加载配置与网段，NetBird 段置顶；未选网段时「下一步」禁用", async () => {
    const wrapper = await openWizard();

    expect(wrapper.find(".wiz-step-1").exists()).toBe(true);
    expect(wrapper.findAll(".seg-row")[0].text()).toContain("NetBird 虚拟网");
    expect((wrapper.find(".wiz-next-btn").element as HTMLButtonElement).disabled).toBe(true);
    expect(wrapper.find(".wiz-hint").text()).toContain("先勾选至少一个网段");
  });

  it("勾 NetBird 段 → 下一步进凭证步；再下一步进完成步（三步走通）", async () => {
    const wrapper = await openWizard();

    await wrapper.findAll(".seg-check")[0].trigger("change");
    await wrapper.find(".wiz-next-btn").trigger("click");
    expect(wrapper.find(".wiz-step-2").exists()).toBe(true);
    expect(wrapper.find(".token-masked").text()).toBe("0123…cdef");

    await wrapper.find(".wiz-next-btn").trigger("click");
    expect(wrapper.find(".wiz-step-3").exists()).toBe(true);
    expect(wrapper.find(".wiz-finish-btn").exists()).toBe(true);
  });

  it("步骤 ① 勾物理网段 → 向导内同样弹硬警示（复用子组件，不复制粘贴）", async () => {
    const wrapper = await openWizard();

    await wrapper.findAll(".seg-check")[1].trigger("change");

    expect(wrapper.find(".seg-confirm").exists()).toBe(true);
    expect(wrapper.find(".seg-confirm").text()).toContain("明文传输");
    expect((wrapper.find(".wiz-next-btn").element as HTMLButtonElement).disabled).toBe(true);
  });

  it("完成：save_webui_config 带 enabled=true 与所选网段，防火墙成功提示 + 地址展示 + 写完成键并关闭", async () => {
    const wrapper = await openWizard();

    await wrapper.findAll(".seg-check")[0].trigger("change");
    await wrapper.find(".wiz-next-btn").trigger("click");
    await wrapper.find(".wiz-next-btn").trigger("click");
    invokeMock.mockClear();
    await wrapper.find(".wiz-finish-btn").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("save_webui_config", {
      input: { enabled: true, segments: ["100.84.0.0/16"], port: 17321 },
    });
    // 收尾视图：防火墙结果 + 完整地址
    expect(wrapper.find(".wiz-fw-ok").text()).toContain("防火墙已放行");
    expect(wrapper.find(".access-url").text()).toContain("#token=");
    // 点「完成」关闭 → 写完成键（下次启动不再弹）
    invokeMock.mockClear();
    await wrapper.find(".wiz-close-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("mark_webui_wizard_done");
    expect(wrapper.emitted("close")).toBeTruthy();
  });

  it("完成但防火墙失败 → 展示现成手动命令；关闭仍写完成键（配置已保存）", async () => {
    const wrapper = await openWizard({ saveOutcome: outcomeFixture(false) });

    await wrapper.findAll(".seg-check")[0].trigger("change");
    await wrapper.find(".wiz-next-btn").trigger("click");
    await wrapper.find(".wiz-next-btn").trigger("click");
    await wrapper.find(".wiz-finish-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".firewall-fail").text()).toContain("UAC 被拒绝");
    expect(wrapper.find(".firewall-manual").text()).toContain("netsh");
    // 地址照常展示（配置已保存、凭证可用）
    expect(wrapper.find(".access-url").exists()).toBe(true);

    invokeMock.mockClear();
    await wrapper.find(".wiz-close-btn").trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("mark_webui_wizard_done");
    expect(wrapper.emitted("close")).toBeTruthy();
  });

  it("保存被拒（校验失败）→ 报错停留，不进收尾视图", async () => {
    const wrapper = await openWizard({ saveError: "启用网页端至少要选择一个受信网段" });

    await wrapper.findAll(".seg-check")[0].trigger("change");
    await wrapper.find(".wiz-next-btn").trigger("click");
    await wrapper.find(".wiz-next-btn").trigger("click");
    await wrapper.find(".wiz-finish-btn").trigger("click");
    await flushPromises();

    expect(wrapper.find(".form-error").text()).toContain("网段");
    expect(wrapper.find(".wiz-finish-btn").exists()).toBe(true);
    expect(wrapper.find(".access-url").exists()).toBe(false);
  });

  it("跳过：不发任何保存，写完成键后关闭（之后可在设置页随时开启）", async () => {
    const wrapper = await openWizard();

    invokeMock.mockClear();
    await wrapper.find(".wiz-skip-btn").trigger("click");
    await flushPromises();

    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "save_webui_config")).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith("mark_webui_wizard_done");
    expect(wrapper.emitted("close")).toBeTruthy();
  });
});
