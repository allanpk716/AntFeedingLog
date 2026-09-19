import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WebUiTokenArea from "./WebUiTokenArea.vue";

const TOKEN = "0123456789abcdef0123456789abcdef";

function mountArea(token = TOKEN, generatedAt: string | null = "2026-09-19 08:00:00") {
  return mount(WebUiTokenArea, {
    props: { token, generatedAt },
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("访问凭证区（webui-checkin 票 03）", () => {
  it("默认打码显示（不露明文）+ 生成时间", () => {
    const wrapper = mountArea();

    const masked = wrapper.find(".token-masked");
    expect(masked.exists()).toBe(true);
    expect(masked.text()).toBe("0123…cdef");
    expect(wrapper.text()).not.toContain(TOKEN);
    expect(wrapper.find(".token-plain").exists()).toBe(false);
    expect(wrapper.find(".token-meta").text()).toContain("生成于 2026-09-19 08:00:00");
    expect(wrapper.find(".token-meta").text()).toContain("不可自定义");
  });

  it("「查看明文」切换显示完整凭证，可再隐藏（验收：打码/明文切换）", async () => {
    const wrapper = mountArea();

    await wrapper.find(".token-toggle").trigger("click");
    expect(wrapper.find(".token-plain").text()).toBe(TOKEN);
    expect(wrapper.find(".token-masked").exists()).toBe(false);

    await wrapper.find(".token-toggle").trigger("click");
    expect(wrapper.find(".token-masked").exists()).toBe(true);
    expect(wrapper.find(".token-plain").exists()).toBe(false);
  });

  it("凭证为空显示「尚未生成」", () => {
    const wrapper = mountArea("", null);
    expect(wrapper.find(".token-meta").text()).toContain("尚未生成");
  });

  it("重生成两段确认：第一次只进确认态并明示旧地址作废，不触发 regenerate", async () => {
    const wrapper = mountArea();

    await wrapper.find(".token-regen").trigger("click");

    const confirmBox = wrapper.find(".regen-confirm");
    expect(confirmBox.exists()).toBe(true);
    expect(confirmBox.text()).toContain("旧访问地址立即作废");
    expect(wrapper.emitted("regenerate")).toBeUndefined();
  });

  it("重生成确认后取消 → 不触发 regenerate、确认框收起", async () => {
    const wrapper = mountArea();

    await wrapper.find(".token-regen").trigger("click");
    await wrapper.find(".regen-cancel").trigger("click");

    expect(wrapper.emitted("regenerate")).toBeUndefined();
    expect(wrapper.find(".regen-confirm").exists()).toBe(false);
  });

  it("第二次点确认 → emit regenerate 一次（验收：只可重生成不可自定义）", async () => {
    const wrapper = mountArea();

    await wrapper.find(".token-regen").trigger("click");
    await wrapper.find(".regen-ok").trigger("click");

    expect(wrapper.emitted("regenerate")!.length).toBe(1);
    expect(wrapper.find(".regen-confirm").exists()).toBe(false);
  });
});
