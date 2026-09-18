import { describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import App from "./App.vue";

// 票 01 冒烟测试不依赖 Tauri 运行时：mock 掉 IPC，健康检查返回 schema 版本 1
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({ schema_version: 1 })),
}));

describe("首页空状态（票 01 冒烟）", () => {
  it("渲染「暂无窝」占位并显示 schema 版本", async () => {
    const wrapper = mount(App);
    await flushPromises();
    expect(wrapper.text()).toContain("暂无窝");
    expect(wrapper.text()).toContain("schema v1");
  });
});
