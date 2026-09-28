<script setup lang="ts">
/**
 * 巢况时间线弹窗（webui-checkin 票 02；照片随票 07/08；时间线改版随
 * checkin-photo-entry 票 03）：新增/编辑/删除一窝的巢况登记（日期、蚁后数、
 * 工蚁数都可空、换巢标记、备注），每条登记带照片上传与缩略图网格。
 * - 顶部双按钮（票 03）：「📷 拍一张」主按钮一步直达（编排同卡片票 02：桌面
 *   pick_photo_files → save_checkin_with_photos，网页端隐藏 capture input →
 *   multipart 创建模式；自动建当天登记、纯照片合法；成功轻提示 + 重拉时间线 +
 *   抛 saved 让外层头像/摘要跟上）；「📝 完整登记」次按钮把原常驻表单改为按需
 *   展开/收起（默认收起；点条目「编辑」自动展开）；
 * - 表单内嵌照片选择区（票 03）：暂存待上传照片（网页端 File + objectURL 缩略、
 *   桌面路径 + 文件名 chip——asset 协议 scope 只放行 photos/，外部路径出不了
 *   缩略图）、可多选可移除，≤9 张/15MB 预检沿用 photos.ts 口径；保存走创建
 *   通道（字段+照片同事务原子落库，纯照片合法；桌面 ipc.ts 全字段入参，网页端
 *   multipart 创建模式字段段见 photos.ts CreateCheckinPhotosFields）；空提交
 *   前端拦截（字段全空且未选照片 → 内联红字，不进轻提示）；编辑态不嵌照片
 *   （照片增删仍走条目既有按钮）；
 * - 时间线左轴视觉（票 03，基线 mock-timeline-styles 形态 2）：竖轴 + 每条
 *   圆点节点，今日节点/日期橙色高亮；日期 + 换巢/基线 chip 挂轴，条目内容套
 *   卡片；纯照片条目（数/换巢/备注全空且带照片）不显示正文行；
 * - 空时间线（票 03，基线 mock-checkin-photo-entry 场景 3）：大占位「📷 拍一张
 *   巢况照片」按钮（同拍照编排）+ 次级链接「或做一次完整登记」（展开表单）；
 * - 时间线按日期倒序（Rust 排好），最早一条标「基线」chip（基线 = 最早登记日期，
 *   纯投影：改首条日期/删首条后 Rust 侧自然顺延）；数值仍防负数，日期可补录
 *   过去，未来日期后端拒绝；
 * - 既有登记条目的照片入口（票 07/08，保留不动）：
 *   「传照片」桌面走 pick_photo_files 系统文件对话框 → attach_photos；
 *   浏览器走隐藏 <input type=file> 双入口（终局评审）：「拍照」带
 *   capture=environment 手机直调相机、「从相册选」不带 capture 弹系统相册，
 *   共用 uploadPhotosHttp（multipart POST /api/photos）——两边都由 Rust 校验
 *   重编码 + 写入协议落库，成功后重拉时间线；**失败分支也重拉**（批量可能
 *   部分成功，已落库的照片立即出现）；
 *   缩略图桌面经 asset 协议读数据目录 photos/（lib/photos.ts photoSrc），浏览器
 *   经 loadPhotoBlobUrl fetch blob（<img> 带不了 Authorization 头，token 不进
 *   URL），组件卸载 revokeObjectUrl 释放；点开大图；文件缺失（库有元数据、
 *   磁盘没文件）显示占位符提示，不崩溃；删除确认文案补「该登记的 N 张照片
 *   将一并删除」；
 * - 大图查看器（票 07）内「调整头像裁剪」（窝头像票 04）：进 PhotoCropEditor
 *   拖动/缩放调裁剪，保存走 update_photo_crop + 轻提示，回写本地时间线；
 *   上传流程零变化（不自动弹编辑器，新照片默认居中）；
 * - 手机竖屏（票 08）：≤480px 视口单列表单 + 大号按钮（.vp-form-stack 媒体查询
 *   落点，断点类名供组件测试断言——jsdom 不套用媒体查询）；
 * - 删除两段确认照 LogListPage 先例；
 * - 巢况永不参与提醒：本组件只发 save/list/update/delete_checkin 与照片通路。
 * 本组件自持状态、自己发 IPC，任何写成功后抛 saved 让外层刷新（卡片摘要即时跟上）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  attachPhotos,
  deleteCheckin,
  getPhotoAbsDir,
  isTauri,
  listCheckins,
  pickPhotoFiles,
  saveCheckin,
  saveCheckinWithPhotos,
  updateCheckin,
} from "../lib/ipc";
import type { Colony, NestCheckin, NestPhotoMeta } from "../types";
import { checkinEntryLine, countsText } from "../lib/checkin";
import {
  MAX_PHOTOS_PER_SUBMIT,
  MAX_PHOTO_BYTES,
  createCheckinPhotosHttp,
  loadPhotoBlobUrl,
  photoSrc,
  revokeObjectUrl,
  uploadPhotosHttp,
} from "../lib/photos";
import { todayIso } from "../lib/dates";
import { showSuccess } from "../lib/toast";
import PhotoCropEditor from "./PhotoCropEditor.vue";

const props = defineProps<{ colony: Colony }>();
const emit = defineEmits<{
  close: [];
  saved: [];
  /** 弹窗内照片已旋转（头像旋转）：冒泡给宿主卡片刷新头像（rel_path 不变，
   * 桌面需破缓存、浏览器需重取 blob——卡片侧自行分流）。 */
  rotatedPhoto: [meta: NestPhotoMeta];
}>();

const entries = ref<NestCheckin[]>([]);
const loading = ref(true);
const loadError = ref("");

// ── 表单（新增/编辑共用；editingId = null 即新增态）──

const editingId = ref<number | null>(null);
const dateInput = ref(todayIso());
const queenInput = ref("");
const workerInput = ref("");
const movedInput = ref(false);
const noteInput = ref("");
const formError = ref("");
const busy = ref(false);

const formTitle = computed(() => (editingId.value === null ? "新增登记" : `编辑登记 #${editingId.value}`));

// ── 表单按需展开（票 03）：顶部「📝 完整登记」切换；默认收起；点条目「编辑」
// 自动展开（startEdit 置 formOpen），取消编辑回新增态仍保持展开（既有行为）──

const formOpen = ref(false);
const formVisible = computed(() => formOpen.value || editingId.value !== null);

function toggleForm() {
  formOpen.value = !formOpen.value;
  formError.value = "";
}

/** 基线 = 时间线里最早登记日期（与 Rust 投影同口径，本地算用于标 chip）。 */
const baselineDate = computed<string | null>(() => {
  if (entries.value.length === 0) return null;
  return entries.value.reduce((min, c) => (c.date < min ? c.date : min), entries.value[0].date);
});

/** 今日判定（票 03 左轴高亮）：登记 date === 本机今天。 */
function isToday(c: NestCheckin): boolean {
  return c.date === todayIso();
}

/** 条目正文（票 03）：数 + 备注（换巢不再进正文——以 chip 挂轴日期旁）。 */
function entryBodyLine(c: NestCheckin): string {
  const parts: string[] = [];
  const counts = countsText(c.queen_count, c.worker_count);
  if (counts !== "") parts.push(counts);
  if (c.note !== "") parts.push(`备注：${c.note}`);
  return parts.join(" · ");
}

/** 纯照片条目（票 03）：数/换巢/备注全空（checkinEntryLine 空串口径）且带
 * 照片 → 不显示正文行，只显示照片区 + 操作按钮。 */
function isPurePhotoEntry(c: NestCheckin): boolean {
  return checkinEntryLine(c) === "" && c.photos.length > 0;
}

/** 数值输入解析：空 = 未数(null)；非负整数合法，其余 NaN 由调用方拦截。
 * （type=number 的 v-model 会把可解析值转成 number，这里统一走 String。） */
function parseCount(v: string | number): number | null {
  const t = String(v).trim();
  if (t === "") return null;
  return Number(t);
}

function resetForm() {
  editingId.value = null;
  dateInput.value = todayIso();
  queenInput.value = "";
  workerInput.value = "";
  movedInput.value = false;
  noteInput.value = "";
  formError.value = "";
  clearStagedPhotos(); // 表单嵌照片区（票 03）：回到新增态一并清掉暂存
}

async function load() {
  try {
    entries.value = await listCheckins({ colonyId: props.colony.id });
    if (!isTauri()) {
      void loadPhotoBlobs(entries.value.flatMap((c) => c.photos));
    }
    loadError.value = "";
  } catch (e) {
    loadError.value = String(e);
  }
}

// ── 照片（webui-checkin 票 07/08）：上传 / 网格 / 大图 / 缺图占位 ──

const photoAbsDir = ref("");
const photoBusy = ref(false);
const photoError = ref("");
/** 图片加载失败的照片 id（文件缺失：库有元数据、磁盘没文件）→ 占位符。 */
const missingPhotoIds = ref<Set<number>>(new Set());
/** 大图查看器当前照片；null = 关闭。 */
const viewerPhoto = ref<NestPhotoMeta | null>(null);
/** 头像裁剪编辑器开合（窝头像票 04）：仅从大图查看器手动进入，上传不自动弹。 */
const cropEditorOpen = ref(false);

/** 浏览器侧 objectURL 表（照片 id → blob: URL）；桌面走 asset 协议不经此。 */
const blobUrls = ref<Record<number, string>>({});

/** 桌面照片 URL 的缓存破除版本号（照片 id → 旋转次数）：文件同路径换内容后
 * WebView2 仍按整 URL 缓存旧图，追加 `?v=` 才换新（asset 协议按路径段解析，
 * 查询串不影响定位）。浏览器端 blob 每次 revoke 重取天然全新，不用它。 */
const photoRev = ref<Record<number, number>>({});

function photoSrcOf(p: NestPhotoMeta): string {
  if (!isTauri()) {
    return blobUrls.value[p.id] ?? "";
  }
  const base = photoSrc(p.rel_path, photoAbsDir.value);
  const rev = photoRev.value[p.id];
  return rev ? `${base}?v=${rev}` : base;
}

/** 浏览器取图（票 08）：逐张 fetch blob（时间线照片量小，串行即可）；
 * 失败标 missing → 「文件缺失」占位，取图成功即解除该 id 的标记（终局评审：
 * 瞬时失败的占位不滞留——重拉后取图成功照常显示）。换数据后释放不再在场的旧 URL。 */
async function loadPhotoBlobs(list: NestPhotoMeta[]) {
  const keep = new Set(list.map((p) => p.id));
  for (const [id, url] of Object.entries(blobUrls.value)) {
    if (!keep.has(Number(id))) {
      revokeObjectUrl(url);
      delete blobUrls.value[Number(id)];
    }
  }
  for (const p of list) {
    if (blobUrls.value[p.id] !== undefined) continue;
    try {
      blobUrls.value[p.id] = await loadPhotoBlobUrl(p.rel_path);
      if (missingPhotoIds.value.has(p.id)) {
        const next = new Set(missingPhotoIds.value);
        next.delete(p.id);
        missingPhotoIds.value = next;
      }
    } catch {
      markPhotoMissing(p.id);
    }
  }
}

onBeforeUnmount(() => {
  for (const url of Object.values(blobUrls.value)) {
    revokeObjectUrl(url);
  }
  blobUrls.value = {};
  clearStagedPhotos(); // 表单暂存缩略（票 03）一并释放
});

type PhotoState = "ok" | "missing" | "unavailable";

/** 缺图三态：ok=有 URL；missing=加载失败（文件缺失）；unavailable=URL 未就绪
 *（桌面照片根目录未就绪/浏览器 blob 还在路上）。后两态都渲染占位符，不崩溃。 */
function photoState(p: NestPhotoMeta): PhotoState {
  if (missingPhotoIds.value.has(p.id)) return "missing";
  return photoSrcOf(p) === "" ? "unavailable" : "ok";
}

function markPhotoMissing(id: number) {
  const next = new Set(missingPhotoIds.value);
  next.add(id);
  missingPhotoIds.value = next;
}

/** 上传成功后的公共收尾：清缺图标记、重拉时间线、通知外层刷新。 */
async function afterPhotosLanded() {
  missingPhotoIds.value = new Set();
  await load();
  emit("saved");
}

/** 裁剪保存回写（窝头像票 04）：返回元数据更新进本地时间线与大图状态（同窝
 * 照片对象替换而非原地遗留），并抛 saved 让外层刷新（首页头像投影跟上）。 */
function onCropSaved(meta: NestPhotoMeta) {
  entries.value = entries.value.map((c) => ({
    ...c,
    photos: c.photos.map((p) => (p.id === meta.id ? { ...p, crop: meta.crop } : p)),
  }));
  if (viewerPhoto.value && viewerPhoto.value.id === meta.id) {
    viewerPhoto.value = { ...viewerPhoto.value, crop: meta.crop };
  }
  emit("saved");
}

/** 照片已旋转（头像旋转：烧进文件）：换新图——桌面破缓存（`?v=` 递增，
 * WebView2 按整 URL 缓存，文件同路径换内容不重载），浏览器弃旧 blob 重取
 * （新 blob URL 天然全新）。元数据行零变化，时间线对象不必替换。 */
function onCropRotated(meta: NestPhotoMeta) {
  if (isTauri()) {
    photoRev.value = { ...photoRev.value, [meta.id]: (photoRev.value[meta.id] ?? 0) + 1 };
  } else {
    const old = blobUrls.value[meta.id];
    if (old) {
      revokeObjectUrl(old);
      delete blobUrls.value[meta.id];
    }
    void loadPhotoBlobs(entries.value.flatMap((c) => c.photos));
  }
  emit("rotatedPhoto", meta); // 宿主卡片刷新头像（rel 不变也要换新图）
}

// ── 桌面上传（票 07）：系统文件对话框 → attach_photos ──

async function addPhotosDesktop(c: NestCheckin) {
  photoError.value = "";
  let picked: string[] | null = null;
  try {
    picked = await pickPhotoFiles();
  } catch (e) {
    photoError.value = String(e);
    return;
  }
  if (!picked || picked.length === 0) return; // 用户取消选文件
  photoBusy.value = true;
  try {
    await attachPhotos({ checkinId: c.id, paths: picked });
    await afterPhotosLanded();
  } catch (e) {
    photoError.value = String(e);
    // 批量可能部分成功（前几张已完整落库）：也重拉时间线让已到的照片出现
    await load();
  } finally {
    photoBusy.value = false;
  }
}

// ── 浏览器上传（票 08）：隐藏 file input → uploadPhotosHttp（multipart
// POST /api/photos，客户端预检 15MB/9 张）。终局评审起双入口：「拍照」带
// capture=environment（手机直调相机）、「从相册选」不带 capture（弹系统相册/
// 文件选择），共用同一处理函数；桌面忽略两个入口（走 pick_photo_files）──

const fileInput = ref<HTMLInputElement | null>(null);
const albumInput = ref<HTMLInputElement | null>(null);
/** 待上传的登记 id（input change 时无从知道点的是哪条，点击时记下）。 */
let uploadTargetId = 0;

function onAddPhotos(c: NestCheckin, source: "camera" | "album" = "camera") {
  if (isTauri()) {
    void addPhotosDesktop(c);
    return;
  }
  uploadTargetId = c.id;
  (source === "album" ? albumInput : fileInput).value?.click();
}

async function onFilesChosen(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  input.value = ""; // 清空：同一批文件二次选择也能触发 change
  if (files.length === 0) return; // 用户取消
  photoError.value = "";
  photoBusy.value = true;
  try {
    await uploadPhotosHttp(uploadTargetId, files);
    await afterPhotosLanded();
  } catch (e) {
    photoError.value = String(e);
    // 批量部分成功也重拉（服务端逐张落库，失败时可能已到几张）
    await load();
  } finally {
    photoBusy.value = false;
  }
}

// ── 顶部「📷 拍一张」一步直达（票 03；编排同卡片票 02 ColonyCard 先例）：
// 桌面 pick_photo_files → save_checkin_with_photos（字段全空、date 缺省=今天，
// 纯照片合法），网页端隐藏 capture input → multipart 创建模式；成功轻提示
// （窗不关视图不变的写操作，全局规范）+ 重拉弹窗内时间线 + 抛 saved（外层
// 头像/摘要投影跟上）；失败内联 photo-error（弹窗内既有错误位），不抛 saved ──

const snapBusy = ref(false);
const snapInput = ref<HTMLInputElement | null>(null);

/** 拍照入口：桌面直接走系统文件选择；网页端点隐藏 capture input（手机直调相机）。 */
function onSnapPhoto() {
  photoError.value = "";
  if (isTauri()) {
    void snapSaveDesktop();
    return;
  }
  snapInput.value?.click();
}

async function snapSaveDesktop() {
  let picked: string[] | null = null;
  try {
    picked = await pickPhotoFiles();
  } catch (e) {
    photoError.value = String(e);
    return;
  }
  if (picked === null || picked.length === 0) return; // 用户取消选文件
  snapBusy.value = true;
  try {
    await saveCheckinWithPhotos({ colonyId: props.colony.id, photoPaths: picked });
    showSuccess(`✓ 已登记到「${props.colony.name}」· 头像已更新`);
    await load();
    emit("saved");
  } catch (e) {
    photoError.value = String(e);
  } finally {
    snapBusy.value = false;
  }
}

/** 网页端拍照编排：File 列表 → multipart 创建模式（colonyId + 今天）。取消
 * （files 空）静默返回；input 值清空保证同一批文件二次选择也触发 change。 */
async function onSnapFilesChosen(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  input.value = "";
  if (files.length === 0) return; // 用户取消
  snapBusy.value = true;
  try {
    await createCheckinPhotosHttp(props.colony.id, files, todayIso());
    showSuccess(`✓ 已登记到「${props.colony.name}」· 头像已更新`);
    await load();
    emit("saved");
  } catch (e) {
    photoError.value = String(e);
  } finally {
    snapBusy.value = false;
  }
}

// ── 表单内嵌照片选择区（票 03）：暂存待上传照片，缩略预览、可多选可移除；
// ≤9 张/15MB 预检沿用 photos.ts 口径（选定时拦截，保存时通道内还有同款兜底）。
// 网页端暂存 File + objectURL 缩略（移除/清空/卸载即释放）；桌面暂存路径 +
// 文件名 chip——asset 协议 scope 只放行 photos/，外部路径出不了缩略图。
// 编辑态不渲染本区（照片增删仍走条目既有按钮，规格 Out of Scope）──

const stagedFiles = ref<File[]>([]);
const stagedPaths = ref<string[]>([]);
/** 暂存缩略（网页端）：objectURL 表，与 stagedFiles 同序。 */
const stagedUrls = ref<string[]>([]);
const formPhotoError = ref("");
const formPhotoInput = ref<HTMLInputElement | null>(null);

/** 暂存张数（按环境取对应暂存列）。 */
const stagedCount = computed(() =>
  isTauri() ? stagedPaths.value.length : stagedFiles.value.length,
);

function clearStagedPhotos() {
  for (const url of stagedUrls.value) {
    revokeObjectUrl(url);
  }
  stagedFiles.value = [];
  stagedPaths.value = [];
  stagedUrls.value = [];
  formPhotoError.value = "";
}

function removeStaged(index: number) {
  if (isTauri()) {
    stagedPaths.value = stagedPaths.value.filter((_, i) => i !== index);
    return;
  }
  revokeObjectUrl(stagedUrls.value[index] ?? "");
  stagedFiles.value = stagedFiles.value.filter((_, i) => i !== index);
  stagedUrls.value = stagedUrls.value.filter((_, i) => i !== index);
}

/** 桌面暂存 chip 的文件名（路径尾段，兼容混合分隔符）。 */
function stagedFileName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path;
}

/** 加照片入口：桌面系统文件选择；网页端点隐藏 file input（不带 capture，
 * 相册/文件选择均可）。 */
function addFormPhotos() {
  formPhotoError.value = "";
  if (isTauri()) {
    void addFormPhotosDesktop();
    return;
  }
  formPhotoInput.value?.click();
}

async function addFormPhotosDesktop() {
  let picked: string[] | null = null;
  try {
    picked = await pickPhotoFiles();
  } catch (e) {
    formPhotoError.value = String(e);
    return;
  }
  if (picked === null || picked.length === 0) return; // 用户取消选文件
  if (stagedPaths.value.length + picked.length > MAX_PHOTOS_PER_SUBMIT) {
    formPhotoError.value = `一次最多上传 ${MAX_PHOTOS_PER_SUBMIT} 张照片`;
    return;
  }
  stagedPaths.value = [...stagedPaths.value, ...picked];
}

async function onFormFilesChosen(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  input.value = ""; // 清空：同一批文件二次选择也能触发 change
  if (files.length === 0) return; // 用户取消
  if (stagedFiles.value.length + files.length > MAX_PHOTOS_PER_SUBMIT) {
    formPhotoError.value = `一次最多上传 ${MAX_PHOTOS_PER_SUBMIT} 张照片`;
    return;
  }
  for (const f of files) {
    if (f.size > MAX_PHOTO_BYTES) {
      formPhotoError.value = `${f.name}: 单张照片压缩前不能超过 ${MAX_PHOTO_BYTES / 1024 / 1024}MB`;
      return;
    }
  }
  stagedFiles.value = [...stagedFiles.value, ...files];
  stagedUrls.value = [...stagedUrls.value, ...files.map((f) => URL.createObjectURL(f))];
}

onMounted(async () => {
  // 照片根目录取不到不挡时间线（缩略图走「预览不可用」占位）
  try {
    photoAbsDir.value = await getPhotoAbsDir();
  } catch {
    photoAbsDir.value = "";
  }
  await load();
  loading.value = false;
});

function startEdit(c: NestCheckin) {
  editingId.value = c.id;
  dateInput.value = c.date;
  queenInput.value = c.queen_count === null ? "" : String(c.queen_count);
  workerInput.value = c.worker_count === null ? "" : String(c.worker_count);
  movedInput.value = c.moved_nest;
  noteInput.value = c.note;
  formError.value = "";
  formOpen.value = true; // 编辑既有条目自动展开表单（票 03）
  clearStagedPhotos(); // 新建暂存不串场：编辑态不嵌照片
}

function cancelEdit() {
  resetForm();
}

async function submit() {
  formError.value = "";
  const queen = parseCount(queenInput.value);
  const worker = parseCount(workerInput.value);
  if (Number.isNaN(queen) || (queen !== null && queen < 0)) {
    formError.value = "蚁后数应为非负整数，不数就留空";
    return;
  }
  if (Number.isNaN(worker) || (worker !== null && worker < 0)) {
    formError.value = "工蚁数应为非负整数，不数就留空";
    return;
  }
  const note = noteInput.value.trim();
  // 新建且选了照片 → 走创建通道（字段+照片同事务，纯照片合法——照片算内容，
  // 不再要求字段至少一项）；编辑态不嵌照片，防呆照旧。
  const withPhotos = editingId.value === null && stagedCount.value > 0;
  if (queen === null && worker === null && !movedInput.value && note === "" && !withPhotos) {
    formError.value = "至少填一项：蚁后数 / 工蚁数 / 换巢 / 备注，或选一张照片";
    return;
  }

  busy.value = true;
  try {
    if (withPhotos) {
      if (isTauri()) {
        await saveCheckinWithPhotos({
          colonyId: props.colony.id,
          date: dateInput.value,
          queenCount: queen,
          workerCount: worker,
          movedNest: movedInput.value,
          note: note === "" ? null : note,
          photoPaths: stagedPaths.value,
        });
      } else {
        // 网页端创建模式：字段段随照片一并原子落库（photos.ts 第 4 参，票 03 缝合）
        await createCheckinPhotosHttp(props.colony.id, stagedFiles.value, dateInput.value, {
          queenCount: queen,
          workerCount: worker,
          movedNest: movedInput.value,
          note: note === "" ? null : note,
        });
      }
    } else if (editingId.value === null) {
      await saveCheckin({
        input: {
          colony_id: props.colony.id,
          date: dateInput.value,
          queen_count: queen,
          worker_count: worker,
          moved_nest: movedInput.value,
          note: note === "" ? null : note,
        },
      });
    } else {
      await updateCheckin({
        id: editingId.value,
        input: {
          date: dateInput.value,
          queen_count: queen,
          worker_count: worker,
          moved_nest: movedInput.value,
          note: note === "" ? null : note,
        },
      });
    }
    resetForm();
    await load();
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}

// ── 删除（两段确认：第一次进入确认态，第二次才真删；带照片的登记补一句
// 「该登记的 N 张照片将一并删除」——Rust 侧按「先库后文件」顺序连带删照片）──

const confirmDeleteId = ref<number | null>(null);

function deletePhotoWarning(c: NestCheckin): string {
  return c.photos.length > 0 ? `该登记的 ${c.photos.length} 张照片将一并删除` : "";
}

async function requestDelete(c: NestCheckin) {
  if (confirmDeleteId.value !== c.id) {
    confirmDeleteId.value = c.id;
    return;
  }
  confirmDeleteId.value = null;
  try {
    await deleteCheckin({ id: c.id });
    if (editingId.value === c.id) {
      resetForm();
    }
    await load();
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  }
}
</script>

<template>
  <div class="overlay" @click.self="$emit('close')">
    <div class="dialog checkin-dialog vp-form-stack">
      <h3>巢况时间线 · {{ colony.name }}</h3>

      <!-- 浏览器照片上传（票 08；终局评审起双入口）：拍照 input 带
           capture=environment（手机直调相机），从相册选 input 不带 capture；
           桌面忽略这两个 input（走 pick_photo_files 系统对话框） -->
      <input
        ref="fileInput"
        class="photo-file-input"
        type="file"
        accept="image/*"
        multiple
        capture="environment"
        @change="onFilesChosen"
      />
      <input
        ref="albumInput"
        class="photo-album-input"
        type="file"
        accept="image/*"
        multiple
        @change="onFilesChosen"
      />
      <!-- 「📷 拍一张」入口（票 03，网页端）：capture input 同上；桌面忽略 -->
      <input
        ref="snapInput"
        class="snap-file-input"
        type="file"
        accept="image/*"
        multiple
        capture="environment"
        @change="onSnapFilesChosen"
      />
      <!-- 表单嵌照片区入口（票 03，网页端）：不带 capture（相册/文件均可） -->
      <input
        ref="formPhotoInput"
        class="form-photo-input"
        type="file"
        accept="image/*"
        multiple
        @change="onFormFilesChosen"
      />

      <p v-if="loadError" class="form-error">{{ loadError }}</p>
      <p v-if="photoError" class="form-error photo-error">{{ photoError }}</p>

      <!-- 顶部双按钮（票 03）：拍照主导，完整登记按需展开；空态时由占位区的
           大按钮/链接承担同款入口（对齐 mock 场景 3，避免双份拍照按钮） -->
      <div v-if="!loading && entries.length > 0" class="top-actions">
        <button
          class="btn-snap snap-btn"
          type="button"
          :disabled="snapBusy"
          title="拍一张自动登记到今天，头像即时更新"
          @click="onSnapPhoto"
        >
          {{ snapBusy ? "处理中…" : "📷 拍一张" }}
          <small>自动建今天的登记 · 头像跟着换</small>
        </button>
        <button class="form-toggle-btn" type="button" @click="toggleForm">
          {{ formOpen && editingId === null ? "收起" : "📝 完整登记" }}
        </button>
      </div>

      <div class="timeline-wrap">
        <p v-if="loading" class="checkin-empty">加载中…</p>
        <!-- 空态占位（票 03，基线 mock 场景 3）：拍一张是主角，次级链接展开表单 -->
        <div v-else-if="entries.length === 0" class="empty-snap">
          <button
            class="empty-snap-btn"
            type="button"
            :disabled="snapBusy"
            @click="onSnapPhoto"
          >
            {{ snapBusy ? "处理中…" : "📷 拍一张巢况照片" }}
          </button>
          <div class="empty-snap-hint">
            拍一张就能开始记录，头像也会用它<br />
            蚁后数 / 工蚁数 / 备注之后随时能补
          </div>
          <button class="empty-full-link" type="button" @click="formOpen = true">
            或做一次完整登记（数蚁口 + 备注）→
          </button>
        </div>
        <!-- 滚动外移到 .timeline-scroll：左轴 ::before 随内容全长，不随滚动截断 -->
        <div v-else class="timeline-scroll">
          <ol class="timeline">
            <li
              v-for="c in entries"
              :key="c.id"
              class="entry node"
              :class="{ today: isToday(c) }"
              :data-checkin-id="c.id"
            >
              <!-- 日期 + 换巢/基线标记挂轴（票 03 左轴视觉） -->
              <div class="entry-head">
                <span class="entry-date">{{ isToday(c) ? `今天 ${c.date}` : c.date }}</span>
                <span v-if="c.moved_nest" class="moved-chip">换巢</span>
                <span v-if="c.date === baselineDate" class="baseline-chip">基线</span>
              </div>
              <div class="entry-card">
                <!-- 纯照片条目（数/换巢/备注全空且带照片）无正文行（票 03）；
                     全空且无照片的既有边界行仍给「（未填内容）」兜底 -->
                <div v-if="entryBodyLine(c) !== ''" class="entry-main">{{ entryBodyLine(c) }}</div>
                <div
                  v-else-if="!isPurePhotoEntry(c) && !c.moved_nest"
                  class="entry-main entry-main-ghost"
                >
                  （未填内容）
                </div>
                <div v-if="c.photos.length > 0" class="entry-photos">
                  <template v-for="p in c.photos" :key="p.id">
                    <img
                      v-if="photoState(p) === 'ok'"
                      class="photo-thumb"
                      :src="photoSrcOf(p)"
                      :title="p.original_name ?? p.rel_path"
                      alt="巢况照片缩略图"
                      @error="markPhotoMissing(p.id)"
                      @click="viewerPhoto = p"
                    />
                    <div
                      v-else-if="photoState(p) === 'missing'"
                      class="photo-thumb photo-missing"
                      :data-photo-id="p.id"
                      title="库里有这条照片，但磁盘上找不到文件（可能恢复过旧备份）"
                    >
                      文件缺失
                    </div>
                    <div v-else class="photo-thumb photo-unavailable" title="照片预览暂不可用">
                      预览不可用
                    </div>
                  </template>
                </div>
                <div class="entry-ops">
                  <button
                    class="entry-btn photo-add-btn"
                    type="button"
                    :disabled="photoBusy"
                    title="选择照片（自动压缩：长边 2048、JPEG，原图与 GPS 信息不留）"
                    @click="onAddPhotos(c)"
                  >
                    {{ photoBusy ? "处理中…" : (isTauri() ? "传照片" : "拍照") }}
                  </button>
                  <!-- 手机第二入口（终局评审）：从相册选（桌面不渲染，桌面单入口走系统文件对话框） -->
                  <button
                    v-if="!isTauri()"
                    class="entry-btn photo-album-btn"
                    type="button"
                    :disabled="photoBusy"
                    title="从相册选择照片（自动压缩同拍照）"
                    @click="onAddPhotos(c, 'album')"
                  >
                    从相册选
                  </button>
                  <button class="entry-btn entry-edit-btn" type="button" @click="startEdit(c)">
                    编辑
                  </button>
                  <button
                    class="entry-btn entry-delete-btn"
                    :class="{ confirming: confirmDeleteId === c.id }"
                    type="button"
                    @click="requestDelete(c)"
                  >
                    {{ confirmDeleteId === c.id ? "确认删除？" : "删除" }}
                  </button>
                </div>
                <p
                  v-if="confirmDeleteId === c.id && c.photos.length > 0"
                  class="delete-photo-warn"
                >
                  {{ deletePhotoWarning(c) }}
                </p>
              </div>
            </li>
          </ol>
        </div>
      </div>

      <!-- 完整登记表单（票 03 起按需展开：顶部「📝 完整登记」或空态次级链接
           或条目「编辑」唤出；编辑态不嵌照片区） -->
      <div v-if="formVisible">
        <div class="form-head">
          <span class="form-title">{{ formTitle }}</span>
          <button
            v-if="editingId !== null"
            class="entry-btn cancel-edit-btn"
            type="button"
            @click="cancelEdit"
          >
            取消编辑
          </button>
          <button
            v-else
            class="entry-btn form-collapse-btn"
            type="button"
            @click="toggleForm"
          >
            收起
          </button>
        </div>

        <div class="form-grid">
          <div>
            <div class="field-label">日期（默认今天，可补录过去）</div>
            <input v-model="dateInput" class="date-input" type="date" />
          </div>
          <div class="num-fields">
            <div>
              <div class="field-label">蚁后数（可空）</div>
              <input
                v-model="queenInput"
                class="queen-input"
                type="number"
                min="0"
                step="1"
                placeholder="未数"
              />
            </div>
            <div>
              <div class="field-label">工蚁数（可空）</div>
              <input
                v-model="workerInput"
                class="worker-input"
                type="number"
                min="0"
                step="1"
                placeholder="未数"
              />
            </div>
          </div>
        </div>

        <label class="moved-row">
          <input v-model="movedInput" class="moved-input" type="checkbox" />
          换巢了
        </label>

        <!-- 表单嵌照片区（票 03，基线 mock 场景 3）：照片是表单的一部分，
             保存随登记一并上传；编辑态不渲染（照片增删走条目既有按钮） -->
        <div v-if="editingId === null" class="photo-pick">
          <template v-if="isTauri()">
            <span
              v-for="(p, i) in stagedPaths"
              :key="`${p}-${i}`"
              class="staged-item staged-name-item"
            >
              <span class="staged-name">{{ stagedFileName(p) }}</span>
              <button
                class="staged-remove"
                type="button"
                title="移除这张照片"
                @click="removeStaged(i)"
              >
                ×
              </button>
            </span>
          </template>
          <template v-else>
            <span v-for="(url, i) in stagedUrls" :key="url" class="staged-item">
              <img class="staged-thumb" :src="url" alt="待上传照片缩略图" />
              <button
                class="staged-remove"
                type="button"
                title="移除这张照片"
                @click="removeStaged(i)"
              >
                ×
              </button>
            </span>
          </template>
          <button
            class="staged-add"
            type="button"
            title="拍照或从相册选（可多选，保存时一并上传）"
            @click="addFormPhotos"
          >
            ＋
          </button>
          <span class="photo-pick-note">
            照片随登记一起保存（自动压缩）<br />
            只拍照不填数？用顶部的「📷 拍一张」更快
          </span>
        </div>
        <p v-if="formPhotoError" class="form-error form-photo-error">{{ formPhotoError }}</p>

        <div class="field-label">备注（可选）</div>
        <textarea v-model="noteInput" class="note-input" placeholder="如：新后产卵第一批"></textarea>

        <p v-if="formError" class="form-error">{{ formError }}</p>

        <div class="dlg-btns form-save-btns">
          <button class="btn primary record-btn" type="button" :disabled="busy" @click="submit">
            {{ editingId === null ? "登记" : "保存" }}
          </button>
        </div>
      </div>

      <div class="dlg-btns close-row">
        <button class="btn cancel-btn" type="button" @click="$emit('close')">关闭</button>
      </div>

      <!-- 大图查看器（票 07）：点击缩略图打开，点遮罩关闭；「调整头像裁剪」
           进裁剪编辑器（窝头像票 04），编辑器盖在其上、关闭即回到大图 -->
      <div v-if="viewerPhoto" class="photo-viewer" @click.self="viewerPhoto = null">
        <img
          class="photo-viewer-img"
          :src="photoSrcOf(viewerPhoto)"
          :alt="viewerPhoto.original_name ?? viewerPhoto.rel_path"
        />
        <p class="photo-viewer-name">{{ viewerPhoto.original_name || viewerPhoto.rel_path }}</p>
        <button
          class="viewer-crop-btn"
          type="button"
          @click="cropEditorOpen = true"
        >
          调整头像裁剪
        </button>
      </div>

      <!-- 头像裁剪编辑器（窝头像票 04）：仅手动进入；保存回写本地时间线并抛
           saved 让外层刷新（首页头像投影跟上）；rotated 换新图（头像旋转）；
           上传流程零变化不自动弹 -->
      <PhotoCropEditor
        v-if="cropEditorOpen && viewerPhoto"
        :photo="viewerPhoto"
        :src="photoSrcOf(viewerPhoto)"
        @close="cropEditorOpen = false"
        @saved="onCropSaved"
        @rotated="onCropRotated"
      />
    </div>
  </div>
</template>

<style scoped>
/* 弹窗骨架沿 QuickLogDialog / HibernationDialog 同款（视觉基线 mock-a-light） */
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.dialog {
  width: 470px;
  max-width: 94vw;
  max-height: 88vh;
  overflow-y: auto;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.dialog h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

/* ── 顶部双按钮（票 03；视觉基线 mock-checkin-photo-entry 场景 2）── */
.top-actions {
  display: flex;
  gap: 8px;
  margin-bottom: 10px;
}

.btn-snap {
  flex: 1.6;
  border: 1px solid var(--accent);
  background: var(--accent);
  color: #fff;
  font: inherit;
  font-size: 14px;
  font-weight: 700;
  padding: 9px 8px;
  border-radius: 11px;
  cursor: pointer;
  text-align: center;
}

.btn-snap small {
  display: block;
  font-size: 10px;
  font-weight: 400;
  opacity: 0.85;
}

.btn-snap:hover {
  background: var(--accent-deep);
  border-color: var(--accent-deep);
}

.btn-snap:disabled {
  opacity: 0.6;
  cursor: default;
}

.form-toggle-btn {
  flex: 1;
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  padding: 7px;
  border-radius: 11px;
  cursor: pointer;
}

.form-toggle-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

/* ── 时间线 ── */
.timeline-wrap {
  margin-bottom: 8px;
}

/* 左轴时间线（票 03；视觉基线 mock-timeline-styles 形态 2）：滚动放外层，
   竖轴挂在 .timeline 的 ::before 上随内容全长（放滚动容器内会被视口截断） */
.timeline-scroll {
  max-height: 300px;
  overflow-y: auto;
}

.timeline {
  list-style: none;
  margin: 0;
  padding: 0 0 0 20px;
  position: relative;
}

.timeline::before {
  content: "";
  position: absolute;
  left: 6px;
  top: 8px;
  bottom: 8px;
  width: 2px;
  background: var(--border-strong);
  border-radius: 2px;
}

/* 条目 = 轴上节点：圆点 ::before，今日橙色高亮；卡片样式移入 .entry-card */
.entry {
  position: relative;
  padding: 0 0 12px;
}

.entry::before {
  content: "";
  position: absolute;
  left: -18.5px;
  top: 6px;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--card);
  border: 2.5px solid var(--muted);
}

.entry.today::before {
  border-color: var(--accent);
  background: var(--accent);
}

/* 日期 + 换巢/基线标记挂轴 */
.entry-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11.5px;
  font-weight: 700;
  color: var(--muted);
}

.entry.today .entry-head {
  color: var(--accent-deep);
}

.entry-date {
  font-weight: 700;
  font-size: 11.5px;
}

.moved-chip {
  font-size: 10px;
  padding: 0 8px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent-deep);
  font-weight: 600;
}

.baseline-chip {
  font-size: 10px;
  padding: 0 8px;
  border-radius: 999px;
  background: var(--hib-soft);
  color: var(--hib);
  font-weight: 600;
}

.entry-card {
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--tile);
  padding: 8px 12px;
  margin-top: 3px;
}

.entry-main {
  margin-top: 3px;
  font-size: 13px;
}

.entry-main-ghost {
  color: var(--muted);
}

/* ── 照片网格（票 07）── */
.entry-photos {
  margin-top: 6px;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.photo-thumb {
  width: 72px;
  height: 72px;
  object-fit: cover;
  border-radius: 8px;
  border: 1px solid var(--border);
  cursor: zoom-in;
  background: var(--tile);
}

div.photo-thumb {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: var(--muted);
  cursor: default;
  text-align: center;
  padding: 2px;
  box-sizing: border-box;
}

div.photo-missing {
  border-style: dashed;
  border-color: var(--bad);
  color: var(--bad);
}

.delete-photo-warn {
  margin-top: 4px;
  font-size: 12px;
  color: var(--bad);
}

/* 大图查看器：盖住弹窗（z 高于遮罩），点空白处关闭 */
.photo-viewer {
  position: fixed;
  inset: 0;
  z-index: 60;
  background: var(--overlay);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  cursor: zoom-out;
}

.photo-viewer-img {
  max-width: 92vw;
  max-height: 84vh;
  border-radius: 10px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.4);
}

.photo-viewer-name {
  font-size: 12px;
  color: var(--muted);
  max-width: 90vw;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 大图内「调整头像裁剪」入口（窝头像票 04）：查看器容器是 zoom-out 光标，
   按钮自身恢复可点光标 */
.viewer-crop-btn {
  padding: 7px 18px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.viewer-crop-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.entry-ops {
  margin-top: 6px;
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.entry-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--muted);
  font: inherit;
  font-size: 11px;
  padding: 1px 10px;
  border-radius: 8px;
  cursor: pointer;
}

.entry-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.entry-delete-btn.confirming {
  border-color: var(--bad);
  background: var(--bad-soft);
  color: var(--bad);
  font-weight: 600;
}

.checkin-empty {
  margin: 6px 0 10px;
  text-align: center;
  font-size: 13px;
  color: var(--muted);
  border: 1.5px dashed var(--border-strong);
  border-radius: 10px;
  padding: 14px;
}

/* ── 空态占位（票 03；视觉基线 mock-checkin-photo-entry 场景 3）：拍一张是主角 ── */
.empty-snap {
  border: 2px dashed var(--border-strong);
  border-radius: 14px;
  padding: 22px 14px;
  text-align: center;
  background: var(--tile);
  margin-bottom: 8px;
}

.empty-snap-btn {
  border: 1px solid var(--accent);
  background: var(--accent);
  color: #fff;
  font: inherit;
  font-size: 16px;
  font-weight: 700;
  padding: 12px 26px;
  border-radius: 12px;
  cursor: pointer;
}

.empty-snap-btn:hover {
  background: var(--accent-deep);
  border-color: var(--accent-deep);
}

.empty-snap-btn:disabled {
  opacity: 0.6;
  cursor: default;
}

.empty-snap-hint {
  font-size: 12px;
  color: var(--muted);
  margin-top: 8px;
}

.empty-full-link {
  border: none;
  background: none;
  color: var(--accent-deep);
  font: inherit;
  font-size: 12.5px;
  cursor: pointer;
  text-decoration: underline;
  margin-top: 6px;
}

/* ── 表单 ── */
.form-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-top: 1px solid var(--border);
  padding-top: 10px;
}

.form-title {
  font-size: 13px;
  font-weight: 700;
}

.form-grid {
  display: flex;
  gap: 12px;
}

.form-grid > div {
  flex: 1;
}

.num-fields {
  display: flex;
  gap: 8px;
}

.num-fields > div {
  flex: 1;
}

.field-label {
  font-size: 12px;
  color: var(--muted);
  margin: 10px 0 6px;
}

.dialog input[type="date"],
.dialog input[type="number"],
.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
  box-sizing: border-box;
}

.dialog textarea {
  height: 52px;
  resize: none;
  margin-top: 2px;
}

.moved-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 10px;
  font-size: 13px;
  cursor: pointer;
}

.form-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
}

.dlg-btns {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}

/* 票 03：保存行收进表单块、关闭行常驻底部——两行靠近些 */
.form-save-btns {
  margin-top: 10px;
}

.close-row {
  margin-top: 10px;
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

/* 隐藏的浏览器上传入口（票 08 + 票 03 拍一张/表单嵌照片）：视觉隐藏但可
   programmatic click */
.photo-file-input,
.photo-album-input,
.snap-file-input,
.form-photo-input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
}

/* ── 表单嵌照片区（票 03；视觉基线 mock-checkin-photo-entry 场景 3）── */
.photo-pick {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
  margin-top: 10px;
}

.staged-item {
  position: relative;
  display: inline-flex;
}

.staged-thumb {
  width: 58px;
  height: 58px;
  border-radius: 8px;
  object-fit: cover;
  border: 1px solid var(--border-strong);
  display: block;
}

.staged-name-item {
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  padding: 4px 8px;
  max-width: 150px;
}

.staged-name {
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.staged-remove {
  position: absolute;
  top: -6px;
  right: -6px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--muted);
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
  padding: 0;
}

.staged-remove:hover {
  border-color: var(--bad);
  color: var(--bad);
}

.staged-add {
  width: 58px;
  height: 58px;
  border: 1.5px dashed var(--border-strong);
  border-radius: 8px;
  background: none;
  font-size: 20px;
  color: var(--muted);
  cursor: pointer;
  flex: none;
}

.staged-add:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.photo-pick-note {
  font-size: 11px;
  color: var(--muted);
  flex: 1;
  min-width: 120px;
}

/* ── 手机竖屏（票 08）：≤480px 单列表单 + 大号可点目标；vp-form-stack 是
   媒体查询落点（断点类名供组件测试断言——jsdom 不套用媒体查询）── */
@media (max-width: 480px) {
  .vp-form-stack.dialog {
    padding: 12px;
    max-height: 92vh;
  }

  .vp-form-stack .form-grid {
    flex-direction: column;
    gap: 0;
  }

  .vp-form-stack input[type="date"],
  .vp-form-stack input[type="number"],
  .vp-form-stack textarea {
    padding: 11px 12px;
    font-size: 16px; /* ≥16px 防 iOS 聚焦自动放大 */
  }

  .vp-form-stack .entry-btn {
    padding: 6px 14px;
    font-size: 13px;
  }

  /* 顶部双按钮（票 03）：手机上大号可点目标（对齐 mock 场景 2 手机壳） */
  .vp-form-stack .btn-snap {
    padding: 11px 10px;
    font-size: 15px;
  }

  .vp-form-stack .form-toggle-btn {
    padding: 11px 8px;
    font-size: 14px;
  }

  .vp-form-stack .empty-snap-btn {
    padding: 12px 22px;
    font-size: 16px;
  }

  .vp-form-stack .staged-thumb,
  .vp-form-stack .staged-add {
    width: 66px;
    height: 66px;
  }

  .vp-form-stack .btn {
    padding: 11px 22px;
    font-size: 15px;
  }

  .vp-form-stack .photo-thumb {
    width: 88px;
    height: 88px;
  }

  .vp-form-stack .dlg-btns {
    flex-direction: row-reverse;
  }
}
</style>
