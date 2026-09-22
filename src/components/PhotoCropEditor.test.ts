import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import PhotoCropEditor from "./PhotoCropEditor.vue";
import { resetAvatarShapeForTests } from "../lib/ipc"; // 形状镜像透传（ipcMock 原样放行非命令导出）
import type { NestPhotoMeta } from "../types";

// 不依赖 Tauri 运行时：统一 mock 调用层（沿 NestCheckinDialog.test.ts 先例）
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("../lib/ipc", async (importOriginal) => {
  const { ipcModuleMock } = await import("../testing/ipcMock");
  return ipcModuleMock(invokeMock)(importOriginal);
});
// 轻提示接线（沿 LocationManagerPanel.test.ts 先例）：toast 模块整体 mock，
// 断言调用点弹没弹、弹的什么；store/宿主行为在 toast.test.ts / ToastHost.test.ts
const { showSuccessMock, showErrorMock } = vi.hoisted(() => ({
  showSuccessMock: vi.fn(),
  showErrorMock: vi.fn(),
}));
vi.mock("../lib/toast", () => ({
  showSuccess: showSuccessMock,
  showError: showErrorMock,
}));

const FRAME = 300; // 测试注入的裁剪框像素尺寸（拖动/缩放按它换算归一化增量）

function photo(overrides: Partial<NestPhotoMeta> = {}): NestPhotoMeta {
  return {
    id: 11,
    checkin_id: 7,
    rel_path: "1/x.jpg",
    original_name: "x.jpg",
    note: "",
    crop: null,
    ...overrides,
  };
}

/** 挂载并注入图片自然尺寸与裁剪框 rect（happy-dom 无真实布局，打桩后事件数学可测）。 */
async function mountEditor(p: NestPhotoMeta, imgW: number, imgH: number) {
  const wrapper = mount(PhotoCropEditor, {
    props: { photo: p, src: "http://asset.localhost/x.jpg" },
  });
  const img = wrapper.find("img.crop-img");
  Object.defineProperty(img.element, "naturalWidth", { value: imgW, configurable: true });
  Object.defineProperty(img.element, "naturalHeight", { value: imgH, configurable: true });
  await img.trigger("load");
  const frame = wrapper.find(".crop-frame");
  frame.element.getBoundingClientRect = () =>
    ({
      width: FRAME,
      height: FRAME,
      top: 0,
      left: 0,
      right: FRAME,
      bottom: FRAME,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
  return wrapper;
}

/** 指针事件打桩：MouseEvent 载荷 + defineProperty pointerId
 *（不依赖 happy-dom 的 PointerEvent 实现；元素级 move/up 监听直接命中）。 */
function firePointer(el: Element, type: string, x: number, y: number, id = 1) {
  const ev = new MouseEvent(type, { clientX: x, clientY: y, bubbles: true, cancelable: true });
  Object.defineProperty(ev, "pointerId", { value: id });
  el.dispatchEvent(ev);
}

function fireWheel(el: Element, deltaY: number) {
  const ev = new MouseEvent("wheel", { bubbles: true, cancelable: true });
  Object.defineProperty(ev, "deltaY", { value: deltaY });
  el.dispatchEvent(ev);
}

/** 预览图内联样式（断言归一化映射的落点；沿仓库 element 断言先例做类型收窄）。 */
function imgStyleOf(w: { find: (sel: string) => { element: Element } }) {
  return (w.find("img.crop-img").element as HTMLImageElement).style;
}

/** 当前状态的保存载荷（点保存后 update_photo_crop 收到的 crop）。 */
async function saveCrop(w: ReturnType<typeof mountEditor> extends Promise<infer T> ? T : never) {
  await w.find(".crop-save-btn").trigger("click");
  await flushPromises();
  const call = invokeMock.mock.calls.find(([cmd]) => cmd === "update_photo_crop");
  return call?.[1] as { photoId: number; crop: { x: number; y: number; size: number } | null };
}

beforeEach(() => {
  invokeMock.mockReset();
  showSuccessMock.mockReset();
  showErrorMock.mockReset();
});

describe("PhotoCropEditor（窝头像票 04）", () => {
  it("骨架：方形裁剪框 + 圆形遮罩预览 + 重置/取消/保存按钮", async () => {
    const w = await mountEditor(photo(), 2000, 1000);

    expect(w.find(".crop-frame").exists()).toBe(true);
    expect(w.find(".crop-frame .crop-circle-mask").exists()).toBe(true);
    expect(w.find(".crop-reset-btn").text()).toBe("重置居中");
    expect(w.find(".crop-cancel-btn").text()).toBe("取消");
    expect(w.find(".crop-save-btn").text()).toBe("保存");
    expect(w.find(".crop-zoom").exists()).toBe(true); // 缩放滑杆
  });

  it("图片未加载完成显示等待态，不渲染可交互画布", async () => {
    const w = mount(PhotoCropEditor, {
      props: { photo: photo(), src: "http://asset.localhost/x.jpg" },
    });
    expect(w.find(".crop-loading").exists()).toBe(true);
    expect(w.find(".crop-frame img.crop-img").attributes("style") ?? "").not.toContain("width");
  });

  // ── 归一化映射（协调者裁定唯一口径，横图/竖图各钉一例）：
  //    方形区域左边 = x·W、顶边 = y·H、边长 = size·max(W,H)；
  //    null 默认居中 = 边长短边、长轴居中（2:1 横图即 {0.25, 0, 0.5}）──

  it("横图默认居中：边长=短边、长轴居中（left -50% / width 200%）", async () => {
    const w = await mountEditor(photo({ crop: null }), 2000, 1000);
    const style = imgStyleOf(w);
    expect(style.left).toBe("-50%");
    expect(style.top).toBe("0%");
    expect(style.width).toBe("200%");
    expect(style.height).toBe("100%");
  });

  it("竖图默认居中：top -50% / height 200%（宽高比影响映射的另一半）", async () => {
    const w = await mountEditor(photo({ crop: null }), 1000, 2000);
    const style = imgStyleOf(w);
    expect(style.left).toBe("0%");
    expect(style.top).toBe("-50%");
    expect(style.width).toBe("100%");
    expect(style.height).toBe("200%");
  });

  it("预填照片已有裁剪：按方形区域定位（横图 {0.4,0.2,0.25} = 左 800px、顶 200px、边 500px）", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.4, y: 0.2, size: 0.25 } }), 2000, 1000);
    const style = imgStyleOf(w);
    expect(style.left).toBe("-160%");
    expect(style.top).toBe("-40%");
    expect(style.width).toBe("400%");
    expect(style.height).toBe("200%");
  });

  // ── 拖动 / 缩放（Pointer Events 一套：鼠标单指拖 = 触摸单指拖）──

  it("单指拖动平移：框内右拖 60px（框 300px、边 1000px）→ x 减 0.1；保存载荷与所调一致", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.25, y: 0, size: 0.5 } }), 2000, 1000);
    const frame = w.find(".crop-frame");

    firePointer(frame.element, "pointerdown", 150, 150);
    firePointer(frame.element, "pointermove", 210, 150);
    firePointer(frame.element, "pointerup", 210, 150);

    const arg = await saveCrop(w);
    expect(arg.photoId).toBe(11);
    expect(arg.crop).toEqual({ x: 0.15, y: 0, size: 0.5 });
  });

  it("拖到边界被夹住：载荷恒满足几何域与服务端约束（x ≤ (W-S)/W ≤ 1-size）", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.25, y: 0, size: 0.5 } }), 2000, 1000);
    const frame = w.find(".crop-frame");

    firePointer(frame.element, "pointerdown", 150, 150);
    firePointer(frame.element, "pointermove", -750, 150); // 大幅左拖 → 照片右移到头
    firePointer(frame.element, "pointerup", -750, 150);

    const arg = await saveCrop(w);
    expect(arg.crop!.x).toBe(0.5); // (W-S)/W = 1 - size = 0.5
    expect(arg.crop!.x + arg.crop!.size).toBeLessThanOrEqual(1);
    expect(arg.crop!.y + arg.crop!.size).toBeLessThanOrEqual(1);
    expect(arg.crop!.x).toBeGreaterThanOrEqual(0);
    expect(arg.crop!.y).toBeGreaterThanOrEqual(0);
  });

  it("双指缩放：两指距离翻倍 = 放大 = size 减半（区域中心保持）", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.25, y: 0, size: 0.5 } }), 2000, 1000);
    const frame = w.find(".crop-frame");

    firePointer(frame.element, "pointerdown", 100, 150, 1);
    firePointer(frame.element, "pointerdown", 200, 150, 2);
    firePointer(frame.element, "pointermove", 50, 150, 1);
    firePointer(frame.element, "pointermove", 250, 150, 2);
    firePointer(frame.element, "pointerup", 50, 150, 1);
    firePointer(frame.element, "pointerup", 250, 150, 2);

    const arg = await saveCrop(w);
    // 中心 (0.5, 0.25) 保持，size 0.5 → 0.25
    expect(arg.crop).toEqual({ x: 0.375, y: 0.125, size: 0.25 });
  });

  it("滚轮缩放：deltaY>0 视野外推（size 增大），区域中心保持", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.4, y: 0.2, size: 0.25 } }), 2000, 1000);
    fireWheel(w.find(".crop-frame").element, 120);

    const arg = await saveCrop(w);
    expect(arg.crop!.size).toBeCloseTo(0.2875, 6); // 0.25 × 1.15
    expect(arg.crop!.x).toBeCloseTo(0.38125, 6); // 中心 x=0.525 保持
    expect(arg.crop!.y).toBeCloseTo(0.18125, 6); // 中心 y=0.325 保持
  });

  it("滑杆缩放：zoom=0.5 → size 居整视野(0.5)与下限(0.1)中点 0.3（默认态中心保持）", async () => {
    const w = await mountEditor(photo({ crop: null }), 2000, 1000);
    await w.find(".crop-zoom").setValue("0.5");

    const arg = await saveCrop(w);
    // 默认态 {0.25, 0, 0.5}，中心 (0.5, 0.25) 保持，size 0.5 → 0.3
    expect(arg.crop).toEqual({ x: 0.35, y: 0.1, size: 0.3 });
  });

  // ── 重置居中 / 保存语义 ──

  it("重置居中：回到默认整视野视角（null 的渲染等价态），保存发 crop=null", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.4, y: 0.2, size: 0.25 } }), 2000, 1000);
    await w.find(".crop-reset-btn").trigger("click");

    const style = imgStyleOf(w);
    expect(style.left).toBe("-50%"); // 与 null 默认视角一致
    expect(style.width).toBe("200%");

    const arg = await saveCrop(w);
    expect(arg.crop).toBeNull();
  });

  it("重置后再调整则发具体裁剪（重置语义被后续编辑覆盖）", async () => {
    const w = await mountEditor(photo({ crop: { x: 0.4, y: 0.2, size: 0.25 } }), 2000, 1000);
    await w.find(".crop-reset-btn").trigger("click");
    await w.find(".crop-zoom").setValue("0.5"); // size 0.3，中心 (0.5, 0.25)

    const frame = w.find(".crop-frame");
    firePointer(frame.element, "pointerdown", 150, 150);
    firePointer(frame.element, "pointermove", 210, 150); // 右拖 60px，边 600px → x 减 0.06
    firePointer(frame.element, "pointerup", 210, 150);

    const arg = await saveCrop(w);
    expect(arg.crop).toEqual({ x: 0.29, y: 0.1, size: 0.3 });
  });

  it("未调过的照片直接保存：发 crop=null（不写无意义的默认值）", async () => {
    const w = await mountEditor(photo({ crop: null }), 2000, 1000);
    const arg = await saveCrop(w);
    expect(arg.crop).toBeNull();
  });

  it("保存成功：成功轻提示 + 抛 saved(返回元数据) + 关编辑器", async () => {
    const meta = photo({ crop: { x: 0.15, y: 0, size: 0.5 } });
    invokeMock.mockResolvedValue(meta);
    const w = await mountEditor(photo({ crop: { x: 0.25, y: 0, size: 0.5 } }), 2000, 1000);

    const frame = w.find(".crop-frame");
    firePointer(frame.element, "pointerdown", 150, 150);
    firePointer(frame.element, "pointermove", 210, 150);
    firePointer(frame.element, "pointerup", 210, 150);
    await w.find(".crop-save-btn").trigger("click");
    await flushPromises();

    expect(invokeMock.mock.calls.find(([cmd]) => cmd === "update_photo_crop")![1]).toEqual({
      photoId: 11,
      crop: { x: 0.15, y: 0, size: 0.5 },
    });
    expect(showSuccessMock).toHaveBeenCalledWith("已保存");
    expect(w.emitted("saved")).toEqual([[meta]]);
    expect(w.emitted("close")).toHaveLength(1);
  });

  it("保存失败：失败轻提示带原因，编辑器不关、可重试", async () => {
    invokeMock.mockRejectedValueOnce("照片不存在（可能已被清理）");
    invokeMock.mockResolvedValueOnce(photo({ crop: null }));
    const w = await mountEditor(photo({ crop: null }), 2000, 1000);

    await w.find(".crop-save-btn").trigger("click");
    await flushPromises();
    expect(showErrorMock).toHaveBeenCalledWith("保存失败", "照片不存在（可能已被清理）");
    expect(w.emitted("saved")).toBeUndefined();
    expect(w.emitted("close")).toBeUndefined();
    expect(w.find(".crop-editor").exists()).toBe(true);

    await w.find(".crop-save-btn").trigger("click");
    await flushPromises();
    expect(w.emitted("saved")).toHaveLength(1); // 重试成功
    expect(w.emitted("close")).toHaveLength(1);
  });

  it("取消：不发命令，直接抛 close", async () => {
    const w = await mountEditor(photo({ crop: null }), 2000, 1000);
    await w.find(".crop-cancel-btn").trigger("click");

    expect(invokeMock).not.toHaveBeenCalled();
    expect(w.emitted("close")).toHaveLength(1);
    expect(w.emitted("saved")).toBeUndefined();
  });

  it("预览遮罩跟随全局形状偏好：square 时去圆角、circle 恢复（终局评审集成补）", async () => {
    resetAvatarShapeForTests("square");
    const square = await mountEditor(photo({ crop: null }), 2000, 1000);
    expect(square.find(".crop-circle-mask").classes()).toContain("crop-mask-square");

    resetAvatarShapeForTests(); // 默认 circle
    const circle = await mountEditor(photo({ crop: null }), 2000, 1000);
    expect(circle.find(".crop-circle-mask").classes()).not.toContain("crop-mask-square");
  });
});
