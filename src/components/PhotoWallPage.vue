<script setup lang="ts">
/**
 * 照片墙页（窝头像票 05）：顶层「照片」页，全部窝的照片集中回看。
 * - 数据：消费 photo_wall 只读载荷（窝按 sort/id、窝内按登记日期倒序、同日期
 *   按登记全序、登记内照片按上传序——排序契约后端排好，本组件照载荷顺序渲染，
 *   绝不重排）；「按窝筛选」下拉 = 载荷里的窝（无照片的窝本就不占分组）。
 * - 加载策略（规格 F4 最低策略）：IntersectionObserver 观察缩略图容器，进视口
 *   才解析图片地址——桌面解析 asset URL 直出 + img loading="lazy"；网页端按需
 *   loadPhotoBlobUrl 取 blob（凭证不进 URL），滚出视口释放、换数据释放、卸载
 *   全量释放。取图失败（文件缺失：库有元数据、磁盘没文件）显示「文件缺失」
 *   占位，不崩溃。
 * - 大图查看器：点缩略图打开（本页自持），照片 + 窝名 + 登记日期；上一张/
 *   下一张按列表顺序（新→旧，随当前筛选范围）连翻，越界禁用；网页端翻到
 *   未进过视口的照片按需取 blob。
 * - 「查看登记」：经 list_colonies 取该窝完整对象后在本页渲染 NestCheckinDialog
 *   （弹窗打开是天然反馈，无轻提示）；窝已被删除给失败轻提示兜底。
 * - 只读页：无上传/删除/裁剪任何写入口；页内时间线弹窗自己抛 saved → 重拉
 *   载荷（删了照片墙上即时跟上）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getPhotoAbsDir, isTauri, listColonies, photoWall } from "../lib/ipc";
import type { Colony, NestPhotoMeta, PhotoWallColony } from "../types";
import { loadPhotoBlobUrl, photoSrc, revokeObjectUrl } from "../lib/photos";
import { showError } from "../lib/toast";
import NestCheckinDialog from "./NestCheckinDialog.vue";

/** 扁平列表的一张照片：大图连翻与「查看登记」要的窝名/日期随行携带。 */
interface WallPhoto {
  photo: NestPhotoMeta;
  colonyId: number;
  colonyName: string;
  date: string;
}

const loading = ref(true);
const loadError = ref("");
const wall = ref<PhotoWallColony[]>([]);
/** 按窝筛选键："" = 全部，否则窝 id（select 的 model 双端 DOM 语义不一——
 *  浏览器经 option._value 收原始值、happy-dom 收属性串——统一 Number 归一）。 */
const filterKey = ref<number | string | null>("");
const filterColonyId = computed<number | null>(() =>
  filterKey.value === "" || filterKey.value === null ? null : Number(filterKey.value),
);

async function refresh() {
  viewerIndex.value = null; // 数据可能变了：连翻索引不可跨数据沿用
  try {
    wall.value = await photoWall();
    reconcileBlobs();
    loadError.value = "";
  } catch (e) {
    loadError.value = String(e);
  }
  loading.value = false;
}

/** 窝内全序扁平化（载荷顺序 = 新→旧），大图连翻的唯一顺序依据。 */
const flatPhotos = computed<WallPhoto[]>(() => {
  const out: WallPhoto[] = [];
  for (const c of filteredColonies.value) {
    for (const g of c.groups) {
      for (const chk of g.checkins) {
        for (const p of chk.photos) {
          out.push({ photo: p, colonyId: c.colony_id, colonyName: c.colony_name, date: g.date });
        }
      }
    }
  }
  return out;
});

const filteredColonies = computed<PhotoWallColony[]>(() =>
  filterColonyId.value === null
    ? wall.value
    : wall.value.filter((c) => c.colony_id === filterColonyId.value),
);

// ── 缩略图懒加载（F4）：容器进视口才解析地址；网页端取 blob、离开/换数据释放 ──

const root = ref<HTMLElement | null>(null);
/** 已请求加载的照片 id（进过视口）。ref(Set) 换新对象保响应式。 */
const requestedIds = ref<Set<number>>(new Set());
/** 图片加载失败（文件缺失）的照片 id → 「文件缺失」占位。 */
const missingPhotoIds = ref<Set<number>>(new Set());
/** 网页端 objectURL 表（照片 id → blob: URL）；桌面走 asset 协议不经此。 */
const blobUrls = ref<Record<number, string>>({});
/** 桌面照片根目录（get_photo_abs_dir；取不到不挡页面，缩略图停留占位）。 */
const photoAbsDir = ref("");

const photosById = computed<Map<number, WallPhoto>>(() => {
  const m = new Map<number, WallPhoto>();
  for (const item of flatPhotos.value) m.set(item.photo.id, item);
  return m;
});

function photoUrlOf(p: NestPhotoMeta): string {
  if (!isTauri()) {
    return blobUrls.value[p.id] ?? "";
  }
  return photoSrc(p.rel_path, photoAbsDir.value);
}

type ThumbState = "ok" | "missing" | "pending";

/** 缩略图三态：ok=有地址；missing=加载失败（文件缺失）；pending=还没进视口
 *  （懒加载常态，空占位不写字）或地址未就绪。后两态都不崩溃。 */
function thumbState(p: NestPhotoMeta): ThumbState {
  if (missingPhotoIds.value.has(p.id)) return "missing";
  return requestedIds.value.has(p.id) && photoUrlOf(p) !== "" ? "ok" : "pending";
}

function markPhotoMissing(id: number) {
  const next = new Set(missingPhotoIds.value);
  next.add(id);
  missingPhotoIds.value = next;
}

async function loadBlob(p: NestPhotoMeta) {
  try {
    const url = await loadPhotoBlobUrl(p.rel_path);
    if (requestedIds.value.has(p.id)) {
      blobUrls.value = { ...blobUrls.value, [p.id]: url };
    } else {
      revokeObjectUrl(url); // 加载期间已滚出视口：到货即释放
    }
  } catch {
    markPhotoMissing(p.id);
  }
}

/** 进视口：登记请求——桌面即解析 asset 地址（渲染层响应式跟上）；
 *  网页端异步取 blob。已在requested/已缺图的不再重复发起。 */
function ensureLoaded(id: number) {
  const item = photosById.value.get(id);
  if (!item || requestedIds.value.has(id) || missingPhotoIds.value.has(id)) return;
  const next = new Set(requestedIds.value);
  next.add(id);
  requestedIds.value = next;
  if (!isTauri()) {
    void loadBlob(item.photo);
  }
}

/** 滚出视口（仅网页端释放；桌面 asset URL 无需释放）：撤 URL、退请求位，
 *  重新进视口会再取。 */
function releaseOnLeave(id: number) {
  const url = blobUrls.value[id];
  if (url === undefined) return;
  revokeObjectUrl(url);
  const rest = { ...blobUrls.value };
  delete rest[id];
  blobUrls.value = rest;
  const next = new Set(requestedIds.value);
  next.delete(id);
  requestedIds.value = next;
}

/** 换数据（重拉载荷）释放不在新数据里的 blob（保位已加载的不重取）。 */
function reconcileBlobs() {
  const keep = new Set(flatPhotos.value.map((x) => x.photo.id));
  for (const [id, url] of Object.entries(blobUrls.value)) {
    if (!keep.has(Number(id))) {
      revokeObjectUrl(url);
      const rest = { ...blobUrls.value };
      delete rest[Number(id)];
      blobUrls.value = rest;
    }
  }
}

let io: IntersectionObserver | null = null;
const observedEls = new Set<Element>();

function onIoChange(entries: IntersectionObserverEntry[]) {
  for (const entry of entries) {
    const id = Number(entry.target.getAttribute("data-photo-id"));
    if (!Number.isInteger(id)) continue;
    if (entry.isIntersecting) {
      ensureLoaded(id);
    } else if (!isTauri()) {
      releaseOnLeave(id);
    }
  }
}

/** 观察当前在 DOM 里的缩略图容器（墙加载后/筛选切换后补观察；已移除的退订）。 */
async function observeThumbs() {
  await nextTick();
  if (io === null) return;
  for (const el of [...observedEls]) {
    if (!el.isConnected) {
      io.unobserve(el);
      observedEls.delete(el);
    }
  }
  for (const el of Array.from(root.value?.querySelectorAll("[data-photo-id]") ?? [])) {
    if (observedEls.has(el)) continue;
    io.observe(el);
    observedEls.add(el);
  }
}

// 数据/筛选变化后补观察新出现的缩略图
watch(flatPhotos, () => void observeThumbs());
// 筛选切换：连翻范围变了，正在开着的大图收起
watch(filterColonyId, () => {
  viewerIndex.value = null;
});

onMounted(async () => {
  if (typeof IntersectionObserver !== "undefined") {
    io = new IntersectionObserver(onIoChange);
  }
  if (isTauri()) {
    // 照片根目录取不到不挡页面（缩略图停留占位）
    try {
      photoAbsDir.value = await getPhotoAbsDir();
    } catch {
      photoAbsDir.value = "";
    }
  }
  await refresh();
});

onBeforeUnmount(() => {
  io?.disconnect();
  io = null;
  for (const url of Object.values(blobUrls.value)) {
    revokeObjectUrl(url);
  }
  blobUrls.value = {};
});

// ── 大图查看器（本页自持）：连翻顺序 = 列表顺序（新→旧，随筛选范围） ──

const viewerIndex = ref<number | null>(null);
const viewerItem = computed<WallPhoto | null>(() =>
  viewerIndex.value === null ? null : (flatPhotos.value[viewerIndex.value] ?? null),
);

type ViewerState = "ok" | "missing" | "loading";

function viewerState(): ViewerState {
  const item = viewerItem.value;
  if (item === null) return "loading";
  if (missingPhotoIds.value.has(item.photo.id)) return "missing";
  return photoUrlOf(item.photo) !== "" ? "ok" : "loading";
}

function openViewer(id: number) {
  const idx = flatPhotos.value.findIndex((x) => x.photo.id === id);
  if (idx < 0) return;
  viewerIndex.value = idx;
  ensureLoaded(id);
}

function stepViewer(delta: number) {
  const i = viewerIndex.value;
  if (i === null) return;
  const n = i + delta;
  if (n < 0 || n >= flatPhotos.value.length) return;
  viewerIndex.value = n;
}

// 连翻到未进过视口的照片：按需取（桌面 asset 地址本就直接可解析）
watch(viewerIndex, () => {
  if (viewerItem.value !== null) ensureLoaded(viewerItem.value.photo.id);
});

// ── 「查看登记」：取该窝完整对象后打开巢况时间线（窝已删轻提示兜底） ──

const checkinColony = ref<Colony | null>(null);

async function openCheckin() {
  const item = viewerItem.value;
  if (item === null) return;
  try {
    const cols = await listColonies();
    const c = cols.find((x) => x.id === item.colonyId);
    if (!c) {
      showError("打开巢况登记失败", "该窝可能已被删除");
      return;
    }
    viewerIndex.value = null;
    checkinColony.value = c;
  } catch (e) {
    showError("打开巢况登记失败", String(e));
  }
}

defineExpose({ refresh });
</script>

<template>
  <div ref="root" class="photo-wall">
    <div class="wall-head">
      <h2 class="wall-title">照片墙</h2>
      <label class="filter-label">
        按窝
        <select v-model="filterKey" class="filter-select">
          <option value="">全部窝</option>
          <option v-for="c in wall" :key="c.colony_id" :value="c.colony_id">
            {{ c.colony_name }}
          </option>
        </select>
      </label>
    </div>

    <p v-if="loadError" class="wall-error">{{ loadError }}</p>
    <p v-if="loading" class="wall-empty">加载中…</p>
    <p v-else-if="wall.length === 0" class="wall-empty">
      还没有照片——在巢况登记里给窝拍照后，这里会集中展示
    </p>
    <p v-else-if="filteredColonies.length === 0" class="wall-empty">该窝暂无照片</p>

    <template v-else>
      <section v-for="c in filteredColonies" :key="c.colony_id" class="wall-colony">
        <div class="wall-colony-head">
          <h3 class="wall-colony-name">{{ c.colony_name }}</h3>
          <span class="wall-colony-count">{{ c.groups.reduce((n, g) => n + g.checkins.reduce((m, ck) => m + ck.photos.length, 0), 0) }} 张</span>
          <div class="rule"></div>
        </div>
        <div v-for="g in c.groups" :key="g.date" class="wall-day">
          <div class="wall-day-date">{{ g.date }}</div>
          <div class="wall-thumbs">
            <template v-for="chk in g.checkins" :key="chk.checkin_id">
              <template v-for="p in chk.photos" :key="p.id">
                <div
                  class="wall-thumb"
                  :data-photo-id="p.id"
                  @click="thumbState(p) === 'ok' && openViewer(p.id)"
                >
                  <img
                    v-if="thumbState(p) === 'ok'"
                    :src="photoUrlOf(p)"
                    :alt="p.original_name ?? p.rel_path"
                    loading="lazy"
                    @error="markPhotoMissing(p.id)"
                  />
                  <div
                    v-else-if="thumbState(p) === 'missing'"
                    class="photo-missing"
                    title="库里有这条照片，但磁盘上找不到文件（可能恢复过旧备份）"
                  >
                    文件缺失
                  </div>
                  <div v-else class="thumb-pending" aria-hidden="true"></div>
                </div>
              </template>
            </template>
          </div>
        </div>
      </section>
    </template>

    <!-- 大图查看器：点遮罩或 × 关闭；连翻按列表顺序（新→旧） -->
    <div v-if="viewerItem" class="wall-viewer" @click.self="viewerIndex = null">
      <button class="viewer-btn viewer-close" type="button" aria-label="关闭" @click="viewerIndex = null">
        ×
      </button>
      <img
        v-if="viewerState() === 'ok'"
        class="viewer-img"
        :src="photoUrlOf(viewerItem.photo)"
        :alt="viewerItem.photo.original_name ?? viewerItem.photo.rel_path"
        @error="markPhotoMissing(viewerItem.photo.id)"
      />
      <div v-else-if="viewerState() === 'missing'" class="viewer-hint">文件缺失</div>
      <div v-else class="viewer-hint">加载中…</div>
      <p class="viewer-caption">{{ viewerItem.colonyName }} · {{ viewerItem.date }}</p>
      <div class="viewer-ops">
        <button
          class="viewer-btn viewer-prev"
          type="button"
          :disabled="viewerIndex !== null && viewerIndex <= 0"
          @click="stepViewer(-1)"
        >
          上一张
        </button>
        <button class="viewer-btn viewer-checkin-btn" type="button" @click="openCheckin">
          查看登记
        </button>
        <button
          class="viewer-btn viewer-next"
          type="button"
          :disabled="viewerIndex !== null && viewerIndex >= flatPhotos.length - 1"
          @click="stepViewer(1)"
        >
          下一张
        </button>
      </div>
    </div>

    <!-- 「查看登记」打开的巢况时间线（完整 Colony 经 list_colonies 取得）；
         弹窗内删/改登记抛 saved → 重拉照片墙，墙上即时跟上 -->
    <NestCheckinDialog
      v-if="checkinColony"
      :colony="checkinColony"
      @close="checkinColony = null"
      @saved="void refresh()"
    />
  </div>
</template>

<style scoped>
.photo-wall {
  max-width: 1080px;
  margin: 0 auto;
  padding: 14px 20px 32px;
}

.wall-head {
  display: flex;
  align-items: center;
  gap: 16px;
}

.wall-title {
  font-size: 15px;
  font-weight: 700;
}

.filter-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--muted);
}

.filter-select {
  padding: 5px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--card);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.wall-error {
  margin-top: 14px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--bad-soft);
  color: var(--bad);
  font-size: 13px;
}

.wall-empty {
  margin-top: 60px;
  text-align: center;
  font-size: 16px;
  color: var(--muted);
}

.wall-colony {
  margin-top: 18px;
}

.wall-colony-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 4px;
}

.wall-colony-name {
  font-size: 14px;
}

.wall-colony-count {
  font-size: 12px;
  color: var(--muted);
  background: var(--tile);
  border: 1px solid var(--border);
  padding: 1px 9px;
  border-radius: 999px;
}

.wall-colony-head .rule {
  flex: 1;
  height: 1px;
  background: var(--border);
}

.wall-day {
  margin: 10px 0 16px;
}

.wall-day-date {
  font-size: 12px;
  color: var(--muted);
  margin-bottom: 6px;
}

.wall-thumbs {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 8px;
}

/* 缩略图容器 = 懒加载观察目标（稳定不随状态换元素）；整图 cover 显示，
   照片墙不做裁剪（裁剪只属于头像） */
.wall-thumb {
  position: relative;
  aspect-ratio: 1 / 1;
  border-radius: 10px;
  overflow: hidden;
  background: var(--tile);
  border: 1px solid var(--border);
  cursor: zoom-in;
}

.wall-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.wall-thumb .photo-missing,
.wall-thumb .thumb-pending {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  box-sizing: border-box;
}

.wall-thumb .photo-missing {
  color: var(--bad);
  border: 1px dashed var(--bad);
  border-radius: 10px;
}

/* 大图查看器（沿 NestCheckinDialog 大图层先例）：盖住页面，点空白处关闭 */
.wall-viewer {
  position: fixed;
  inset: 0;
  z-index: 60;
  background: var(--overlay);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  cursor: zoom-out;
}

.viewer-img {
  max-width: 92vw;
  max-height: 78vh;
  border-radius: 10px;
  object-fit: contain;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.4);
}

.viewer-hint {
  color: rgba(255, 255, 255, 0.9);
  font-size: 14px;
}

.viewer-caption {
  color: rgba(255, 255, 255, 0.92);
  font-size: 13px;
}

.viewer-ops {
  display: flex;
  gap: 10px;
}

.viewer-btn {
  padding: 6px 16px;
  border-radius: 9px;
  border: 1px solid rgba(255, 255, 255, 0.45);
  background: rgba(0, 0, 0, 0.35);
  color: #fff;
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.viewer-btn:hover:not(:disabled) {
  border-color: rgba(255, 255, 255, 0.8);
}

.viewer-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.viewer-close {
  position: absolute;
  top: 14px;
  right: 18px;
  font-size: 18px;
  line-height: 1;
  padding: 4px 12px;
}

/* 手机竖屏（≤480px）：缩略图两列（vp-thumbs 媒体查询落点，jsdom 不套用） */
@media (max-width: 480px) {
  .photo-wall {
    padding: 12px 12px 32px;
  }

  .wall-thumbs {
    grid-template-columns: repeat(2, 1fr);
  }
}
</style>
