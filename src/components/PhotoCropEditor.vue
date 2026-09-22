<script setup lang="ts">
/**
 * 头像裁剪编辑器（窝头像票 04）：从巢况时间线大图查看器进入，拖动 + 缩放调
 * 整该照片的归一化方形裁剪，保存走票 01 的 update_photo_crop。
 *
 * 裁剪坐标语义（与票 02 渲染器同一唯一口径，协调者裁定）：
 * crop = { x, y, size }——方形裁剪区域（图内像素）左边 = x·W、顶边 = y·H、
 * 边长 S = size·max(W,H)（size 按图片长边归一）；圆形显示只是方形区域挖角，
 * 与数据无关。默认居中（crop = null）：边长 = 短边、长轴居中，即横图(W>H)
 * size = H/W、x = (W-H)/(2W)、y = 0（竖图对称）。
 * 编辑器内部状态即 {x,y,size}，预览把该方形区域原样铺满裁剪框（img 百分比
 * 缩放 + 负偏移），所见即消费方所得；框实际像素尺寸不影响数学（测试无需
 * 真实布局）。几何夹取 x ≤ (W-S)/W、y ≤ (H-S)/H 天然蕴含服务端约束
 * （x/y/size ∈ [0,1] 且 x+size ≤ 1、y+size ≤ 1）。
 *
 * 交互（Pointer Events 一套通吃鼠标/触摸；touch-action: none 防页面滚动）：
 * 单指/鼠标拖动平移区域；双指距离比缩放（区域中心保持）；滚轮与滑杆缩放。
 * size 域：[sizeMin, sizeMax]，sizeMax = 短/长边（整视野），sizeMin =
 * min(0.1, sizeMax)（近看下限，极端宽高比不越界）。圆形遮罩仅预览，裁剪框
 * 恒为方形。
 *
 * 保存：重置居中且未再编辑 → crop = null（重置居中语义；未调过的照片直接
 * 保存同畴）；否则发构建裁剪（round6 + clamp，x/y+size 超 1 回让 1e-9 防
 * 1 ulp 浮点和被服务端拒——票 01 评审集成提醒）。成功走全局轻提示（改了
 * 不关窗的写操作，即使调的不是当前头像那张也必须有反馈——评审 F5），失败
 * 红色带原因；成功抛 saved(更新后的元数据) 供时间线/大图跟上，随后 close
 * 回到大图。上传流程零变化：本组件只从大图查看器手动进入，不自动弹（D9）。
 */
import { computed, onBeforeUnmount, ref } from "vue";
import { avatarShape, updatePhotoCrop } from "../lib/ipc";
import { showError, showSuccess } from "../lib/toast";
import type { NestPhotoMeta, PhotoCrop } from "../types";

const props = defineProps<{ photo: NestPhotoMeta; src: string }>();
const emit = defineEmits<{ close: []; saved: [meta: NestPhotoMeta] }>();

const MIN_SIZE = 0.1; // 近看下限（长边的 10%）；极端宽高比时被 sizeMax 收紧
const WHEEL_OUT = 1.15; // 滚轮向下 = 视野外推（size 增大）
const WHEEL_IN = 1 / WHEEL_OUT;

// ── 状态：归一化区域 + 图片自然尺寸 ──

/** crop 为空时先占位 0/0/1，图片加载后套默认居中公式（依赖宽高比）。 */
const x = ref(props.photo.crop?.x ?? 0);
const y = ref(props.photo.crop?.y ?? 0);
const size = ref(props.photo.crop?.size ?? 1);
/** 「重置居中」后未再编辑 → 保存发 null；未调过的照片（crop 为空）同畴。 */
const resetPending = ref(props.photo.crop == null);

const imgW = ref(0);
const imgH = ref(0);
const busy = ref(false);

const frameRef = ref<HTMLElement | null>(null);
/** 活动指针表（id → 最近位置）；pointerdown 时缓存框尺寸。 */
const pointers = new Map<number, { x: number; y: number }>();
let pinchBase = 0; // 双指上一次距离（缩放比基准）
let frameW = 0;
let frameH = 0;

const loaded = computed(() => imgW.value > 0 && imgH.value > 0);

/** size 域：整视野上限 = 短/长边；下限 = min(0.1, 上限)。 */
const sizeMax = computed(() => {
  if (!loaded.value) return 1;
  return Math.min(imgW.value, imgH.value) / Math.max(imgW.value, imgH.value);
});
const sizeMin = computed(() => Math.min(MIN_SIZE, sizeMax.value));

/** 滑杆值 = 放大量 [0,1]：0 = 整视野（sizeMax），1 = 最近（sizeMin）。 */
const zoomValue = computed(() =>
  sizeMax.value === sizeMin.value ? 0 : (sizeMax.value - size.value) / (sizeMax.value - sizeMin.value),
);

/** 默认居中（= null 的渲染等价态）：边长短边、长轴居中（协调者裁定公式）。 */
function applyDefaultCenter() {
  const w = imgW.value;
  const h = imgH.value;
  const short = Math.min(w, h);
  size.value = short / Math.max(w, h);
  x.value = (w - short) / 2 / w;
  y.value = (h - short) / 2 / h;
}

/** 区域夹进几何合法域：size ∈ [sizeMin, sizeMax]、x ≤ (W-S)/W、y ≤ (H-S)/H
 *（蕴含服务端 x+size≤1、y+size≤1）。 */
function clampRegion() {
  size.value = Math.min(sizeMax.value, Math.max(sizeMin.value, size.value));
  const s = size.value * Math.max(imgW.value, imgH.value);
  x.value = Math.min(Math.max(x.value, 0), (imgW.value - s) / imgW.value);
  y.value = Math.min(Math.max(y.value, 0), (imgH.value - s) / imgH.value);
}

function onImgLoad(ev: Event) {
  const el = ev.target as HTMLImageElement;
  if (el.naturalWidth > 0 && el.naturalHeight > 0) {
    imgW.value = el.naturalWidth;
    imgH.value = el.naturalHeight;
    if (resetPending.value) applyDefaultCenter(); // 未调过 → 默认居中视角
    else clampRegion(); // 既有裁剪按几何域夹取（服务端合法但可能越图的可视图）
  }
}

/** 预览：把方形区域（左 x·W、顶 y·H、边长 S）原样铺满裁剪框（百分比定位）。 */
const imgStyle = computed(() => {
  if (!loaded.value) return {};
  const s = size.value * Math.max(imgW.value, imgH.value);
  return {
    left: `${(-100 * x.value * imgW.value) / s}%`,
    top: `${(-100 * y.value * imgH.value) / s}%`,
    width: `${(100 * imgW.value) / s}%`,
    height: `${(100 * imgH.value) / s}%`,
  };
});

// ── 拖动 / 缩放（Pointer Events：鼠标单指拖 = 触摸单指拖；双指 = 距离比缩放）──

function pointerDistance(): number {
  const [a, b] = [...pointers.values()];
  return Math.hypot(a!.x - b!.x, a!.y - b!.y);
}

function onPointerDown(ev: PointerEvent) {
  if (!loaded.value || pointers.size >= 2) return; // 第三指忽略
  const el = frameRef.value;
  const rect = el?.getBoundingClientRect();
  if (!el || !rect || rect.width <= 0 || rect.height <= 0) return; // 无布局尺寸不交互
  frameW = rect.width;
  frameH = rect.height;
  // 指针捕获：拖出框外 move/up 仍送达框元素（浏览器标准通路）
  if (typeof el.setPointerCapture === "function") {
    try {
      el.setPointerCapture(ev.pointerId ?? 0);
    } catch {
      // 伪指针 id（测试环境）忽略——元素级监听已足够
    }
  }
  pointers.set(ev.pointerId ?? 0, { x: ev.clientX, y: ev.clientY });
  if (pointers.size === 2) pinchBase = pointerDistance();
}

function onPointerMove(ev: PointerEvent) {
  const p = pointers.get(ev.pointerId ?? 0);
  if (!p) return;
  if (pointers.size === 1) {
    // 平移：框内拖 d px → 区域反向移动 d·S/F 图内像素 → 归一到 x/y
    const dx = ev.clientX - p.x;
    const dy = ev.clientY - p.y;
    if (dx !== 0 || dy !== 0) {
      const s = size.value * Math.max(imgW.value, imgH.value);
      x.value += (-dx * s) / (frameW * imgW.value);
      y.value += (-dy * s) / (frameH * imgH.value);
      clampRegion();
      resetPending.value = false;
    }
  }
  p.x = ev.clientX;
  p.y = ev.clientY;
  if (pointers.size === 2) {
    // 双指：距离比缩放（旧距/新距，张开 = 放大 = size 变小），区域中心保持
    const dist = pointerDistance();
    if (pinchBase > 0 && dist > 0 && dist !== pinchBase) {
      setSize(size.value * (pinchBase / dist));
    }
    pinchBase = dist;
  }
}

function onPointerUp(ev: PointerEvent) {
  pointers.delete(ev.pointerId ?? 0);
  if (pointers.size < 2) pinchBase = 0;
}

function onWheel(ev: WheelEvent) {
  if (!loaded.value) return;
  setSize(size.value * (ev.deltaY > 0 ? WHEEL_OUT : WHEEL_IN));
}

/** 缩放到指定 size，区域中心保持、越界夹住；任何缩放都视为编辑。 */
function setSize(next: number) {
  const cx = x.value + size.value / 2;
  const cy = y.value + size.value / 2;
  size.value = next;
  clampRegion();
  x.value = cx - size.value / 2;
  y.value = cy - size.value / 2;
  clampRegion();
  resetPending.value = false;
}

function onZoomInput(ev: Event) {
  const v = parseFloat((ev.target as HTMLInputElement).value);
  if (!loaded.value || !Number.isFinite(v)) return;
  setSize(sizeMax.value - v * (sizeMax.value - sizeMin.value)); // 0=整视野,1=最近
}

function reset() {
  resetPending.value = true;
  if (loaded.value) applyDefaultCenter();
  else {
    x.value = 0;
    y.value = 0;
    size.value = 1; // 占位；加载后 onImgLoad 套默认居中
  }
}

// ── 保存（票 01 契约 + 票 01 评审的浮点 clamp 集成提醒）──

const round6 = (v: number) => Math.round(v * 1e6) / 1e6;

function buildCrop(): PhotoCrop {
  const s = Math.min(1, Math.max(0, round6(size.value)));
  let px = Math.min(Math.max(round6(x.value), 0), 1 - s);
  if (px + s > 1) px = Math.max(0, 1 - s - 1e-9); // 1 ulp 保险
  let py = Math.min(Math.max(round6(y.value), 0), 1 - s);
  if (py + s > 1) py = Math.max(0, 1 - s - 1e-9);
  return { x: px, y: py, size: s };
}

async function save() {
  if (busy.value) return;
  busy.value = true;
  try {
    const meta = await updatePhotoCrop({
      photoId: props.photo.id,
      crop: resetPending.value ? null : buildCrop(),
    });
    showSuccess("已保存");
    emit("saved", meta);
    emit("close"); // 回到大图查看器
  } catch (e) {
    showError("保存失败", String(e)); // 编辑器不关，可重试
  } finally {
    busy.value = false;
  }
}

onBeforeUnmount(() => {
  pointers.clear();
});
</script>

<template>
  <div class="crop-editor" @click.self="$emit('close')">
    <div class="crop-panel">
      <h3>调整头像裁剪</h3>
      <p class="crop-name">{{ photo.original_name || photo.rel_path }}</p>

      <!-- 方形裁剪框（touch-action: none 防触摸滚动；圆形遮罩仅预览）。
           move/up 挂元素级 + setPointerCapture：拖出框外也不断流 -->
      <div
        ref="frameRef"
        class="crop-frame"
        @pointerdown.prevent="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerUp"
        @wheel.prevent="onWheel"
      >
        <img
          class="crop-img"
          :src="src"
          :alt="photo.original_name ?? photo.rel_path"
          draggable="false"
          :style="imgStyle"
          @load="onImgLoad"
        />
        <p v-if="!loaded" class="crop-loading">加载照片中…</p>
        <div class="crop-circle-mask" :class="{ 'crop-mask-square': avatarShape === 'square' }" aria-hidden="true"></div>
      </div>

      <div class="crop-ops">
        <button class="btn crop-reset-btn" type="button" @click="reset">重置居中</button>
        <label class="crop-zoom-row">
          缩放
          <input
            class="crop-zoom"
            type="range"
            min="0"
            max="1"
            step="0.01"
            :value="zoomValue"
            :disabled="!loaded"
            @input="onZoomInput"
          />
        </label>
      </div>
      <p class="crop-hint">拖动照片调整位置；滚轮 / 滑杆 / 双指缩放；圆形为头像预览</p>

      <div class="crop-btns">
        <button class="btn crop-cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary crop-save-btn" type="button" :disabled="busy" @click="save">
          {{ busy ? "保存中…" : "保存" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.crop-editor {
  position: fixed;
  inset: 0;
  z-index: 70; /* 盖过大图查看器（z-60），关闭即回到大图 */
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
}

.crop-panel {
  width: 400px;
  max-width: 94vw;
  max-height: 92vh;
  overflow-y: auto;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.crop-panel h3 {
  font-size: 15px;
  margin-bottom: 4px;
}

.crop-name {
  font-size: 12px;
  color: var(--muted);
  margin-bottom: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 方形裁剪框：正方形、内容溢出裁掉、触摸不滚屏 */
.crop-frame {
  position: relative;
  width: min(320px, 82vw, 46vh);
  aspect-ratio: 1 / 1;
  overflow: hidden;
  border: 2px solid #fff;
  border-radius: 10px;
  background: #111;
  margin: 0 auto;
  touch-action: none;
  user-select: none;
  cursor: grab;
}

.crop-frame:active {
  cursor: grabbing;
}

.crop-img {
  position: absolute;
  pointer-events: none; /* 指针事件统一落在框上（原生图片拖拽不干扰） */
  -webkit-user-drag: none;
}

.crop-loading {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  color: var(--muted);
  margin: 0;
}

/* 圆形遮罩仅预览（形状偏好切换无需重调裁剪）：圆外压暗 */
.crop-circle-mask {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  box-shadow: 0 0 0 999px rgba(0, 0, 0, 0.45);
  pointer-events: none;
}

/* 全局形状偏好为方形时，预览遮罩跟卡片头像同款方角（终局评审集成补） */
.crop-circle-mask.crop-mask-square {
  border-radius: 8px;
}

.crop-ops {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-top: 12px;
}

.crop-zoom-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--muted);
}

.crop-zoom {
  width: 140px;
  accent-color: var(--accent);
}

.crop-hint {
  margin: 10px 0 0;
  font-size: 11px;
  color: var(--muted);
}

.crop-btns {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 14px;
}

.btn {
  padding: 7px 18px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
  font-weight: 600;
}

.btn.primary:hover {
  background: var(--accent-deep);
}

.btn:disabled {
  opacity: 0.6;
  cursor: default;
}

@media (max-width: 480px) {
  .crop-panel {
    padding: 12px;
  }

  .crop-zoom {
    width: 120px;
  }

  .crop-btns {
    flex-direction: row-reverse;
  }
}
</style>
