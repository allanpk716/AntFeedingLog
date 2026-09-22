<script setup lang="ts">
/**
 * 巢况时间线弹窗（webui-checkin 票 02；照片随票 07/08）：新增/编辑/删除一窝的
 * 巢况登记（日期、蚁后数、工蚁数都可空、换巢标记、备注），每条登记带照片
 * 上传与缩略图网格。
 * - 时间线按日期倒序（Rust 排好），最早一条标「基线」chip（基线 = 最早登记日期，
 *   纯投影：改首条日期/删首条后 Rust 侧自然顺延）；
 * - 至少一项非空才可提交（前端先行拦截，后端兜底）；日期可补录过去，未来日期
 *   后端拒绝；
 * - 照片（票 07 桌面 / 票 08 网页端，同一套组件按环境分流）：
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
  updateCheckin,
} from "../lib/ipc";
import type { Colony, NestCheckin, NestPhotoMeta } from "../types";
import { checkinEntryLine } from "../lib/checkin";
import { loadPhotoBlobUrl, photoSrc, revokeObjectUrl, uploadPhotosHttp } from "../lib/photos";
import { todayIso } from "../lib/dates";
import PhotoCropEditor from "./PhotoCropEditor.vue";

const props = defineProps<{ colony: Colony }>();
const emit = defineEmits<{ close: []; saved: [] }>();

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

/** 基线 = 时间线里最早登记日期（与 Rust 投影同口径，本地算用于标 chip）。 */
const baselineDate = computed<string | null>(() => {
  if (entries.value.length === 0) return null;
  return entries.value.reduce((min, c) => (c.date < min ? c.date : min), entries.value[0].date);
});

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

function photoSrcOf(p: NestPhotoMeta): string {
  if (!isTauri()) {
    return blobUrls.value[p.id] ?? "";
  }
  return photoSrc(p.rel_path, photoAbsDir.value);
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
  if (queen === null && worker === null && !movedInput.value && note === "") {
    formError.value = "至少填一项：蚁后数 / 工蚁数 / 换巢 / 备注";
    return;
  }

  busy.value = true;
  try {
    if (editingId.value === null) {
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

      <p v-if="loadError" class="form-error">{{ loadError }}</p>
      <p v-if="photoError" class="form-error photo-error">{{ photoError }}</p>

      <div class="timeline-wrap">
        <p v-if="loading" class="checkin-empty">加载中…</p>
        <p v-else-if="entries.length === 0" class="checkin-empty">还没有巢况登记</p>
        <ol v-else class="timeline">
          <li
            v-for="c in entries"
            :key="c.id"
            class="entry"
            :data-checkin-id="c.id"
          >
            <div class="entry-head">
              <span class="entry-date">{{ c.date }}</span>
              <span v-if="c.date === baselineDate" class="baseline-chip">基线</span>
            </div>
            <div class="entry-main">{{ checkinEntryLine(c) || "（未填内容）" }}</div>
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
          </li>
        </ol>
      </div>

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

      <div class="field-label">备注（可选）</div>
      <textarea v-model="noteInput" class="note-input" placeholder="如：新后产卵第一批"></textarea>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <div class="dlg-btns">
        <button class="btn cancel-btn" type="button" @click="$emit('close')">关闭</button>
        <button class="btn primary record-btn" type="button" :disabled="busy" @click="submit">
          {{ editingId === null ? "登记" : "保存" }}
        </button>
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
           saved 让外层刷新（首页头像投影跟上）；上传流程零变化不自动弹 -->
      <PhotoCropEditor
        v-if="cropEditorOpen && viewerPhoto"
        :photo="viewerPhoto"
        :src="photoSrcOf(viewerPhoto)"
        @close="cropEditorOpen = false"
        @saved="onCropSaved"
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

/* ── 时间线 ── */
.timeline-wrap {
  margin-bottom: 8px;
}

.timeline {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 300px;
  overflow-y: auto;
}

.entry {
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--tile);
  padding: 8px 12px;
  margin-bottom: 8px;
}

.entry-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.entry-date {
  font-weight: 700;
  font-size: 13px;
}

.baseline-chip {
  font-size: 10px;
  padding: 0 8px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent-deep);
  font-weight: 600;
}

.entry-main {
  margin-top: 3px;
  font-size: 13px;
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

/* 隐藏的浏览器上传入口（票 08）：视觉隐藏但可 programmatic click */
.photo-file-input,
.photo-album-input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
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
