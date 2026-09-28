import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import LoadingHint from "./LoadingHint.vue";

// 展示层测试：组件极简（无 props、无 emits、无业务逻辑），
// 只验全局加载占位的契约——渲染一行「加载中…」文字（界面切换卡顿票 01）。

describe("LoadingHint（全局加载占位组件，界面切换卡顿票 01）", () => {
  it("渲染「加载中…」占位文本，挂 .loading-hint 类", () => {
    const wrapper = mount(LoadingHint);

    expect(wrapper.find(".loading-hint").exists()).toBe(true);
    expect(wrapper.find(".loading-hint").text()).toBe("加载中…");
  });
});
