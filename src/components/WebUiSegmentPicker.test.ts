import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WebUiSegmentPicker from "./WebUiSegmentPicker.vue";
import type { NetworkSegment } from "../types";

const segments: NetworkSegment[] = [
  { cidr: "100.84.0.0/16", encrypted_mesh: true, label: "NetBird 虚拟网" },
  { cidr: "192.168.1.0/24", encrypted_mesh: false, label: null },
  { cidr: "100.64.0.0/10", encrypted_mesh: false, label: null }, // 未确认身份的 CGNAT 段
];

function mountPicker(selected: string[] = []) {
  return mount(WebUiSegmentPicker, {
    props: { segments, selected },
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("受信网段多选（webui-checkin 票 03）", () => {
  it("列出全部网段，NetBird 段带标名与已确认加密徽章，物理段标明文", () => {
    const wrapper = mountPicker();

    const rows = wrapper.findAll(".seg-row");
    expect(rows.length).toBe(3);
    expect(rows[0].find(".seg-cidr").text()).toBe("100.84.0.0/16");
    expect(rows[0].find(".seg-tag").text()).toContain("NetBird 虚拟网");
    expect(rows[0].find(".seg-tag").text()).toContain("已确认加密");
    expect(rows[1].find(".seg-tag").text()).toContain("明文");
    expect(wrapper.find(".seg-empty").exists()).toBe(false);
  });

  it("空列表显示「没有发现可用网段」", () => {
    const wrapper = mount(WebUiSegmentPicker, { props: { segments: [], selected: [] } });
    expect(wrapper.find(".seg-empty").text()).toContain("没有发现可用网段");
  });

  it("勾 NetBird 段（已确认加密身份）即时生效，不需要确认框（验收 1）", async () => {
    const wrapper = mountPicker();

    await wrapper.find(".seg-check").trigger("change");

    expect(wrapper.emitted("update:selected")![0]).toEqual([["100.84.0.0/16"]]);
    expect(wrapper.find(".seg-confirm").exists()).toBe(false);
  });

  it("勾物理网段 → 不勾上，先弹硬警示确认框明示明文传输（验收 1/3）", async () => {
    const wrapper = mountPicker();

    await wrapper.findAll(".seg-check")[1].trigger("change");

    // 不确认不给勾：selected 未变
    expect(wrapper.emitted("update:selected")).toBeUndefined();
    const confirmBox = wrapper.find(".seg-confirm");
    expect(confirmBox.exists()).toBe(true);
    expect(confirmBox.text()).toContain("明文传输");
    expect(confirmBox.text()).toContain("凭证");
  });

  it("物理网段确认框点取消 → 不勾上、确认框收起（验收 3：取消即不勾）", async () => {
    const wrapper = mountPicker();

    await wrapper.findAll(".seg-check")[1].trigger("change");
    await wrapper.find(".confirm-no").trigger("click");

    expect(wrapper.emitted("update:selected")).toBeUndefined();
    expect(wrapper.find(".seg-confirm").exists()).toBe(false);
    expect((wrapper.findAll(".seg-check")[1].element as HTMLInputElement).checked).toBe(false);
  });

  it("物理网段确认框点「确认信任」 → 该段被勾上（含未确认身份的 CGNAT 段）", async () => {
    const wrapper = mountPicker();

    await wrapper.findAll(".seg-check")[2].trigger("change");
    await wrapper.find(".confirm-yes").trigger("click");

    expect(wrapper.emitted("update:selected")![0]).toEqual([["100.64.0.0/10"]]);
    expect(wrapper.find(".seg-confirm").exists()).toBe(false);
  });

  it("取消勾选即时生效（已确认段不再二次询问）", async () => {
    const wrapper = mountPicker(["100.84.0.0/16"]);

    expect((wrapper.findAll(".seg-check")[0].element as HTMLInputElement).checked).toBe(true);
    await wrapper.findAll(".seg-check")[0].trigger("change");

    expect(wrapper.emitted("update:selected")![0]).toEqual([[]]);
  });
});
