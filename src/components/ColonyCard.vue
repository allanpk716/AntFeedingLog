<script setup lang="ts">
/**
 * 窝卡片（交互第三轮 #7 紧凑版，视觉基线 mocks/mock-c-home-cards.html）：
 * 单行头（名字/物种徽章/状态徽章/饲养天数内联），去开始日期行；操作块单行 chip 两列；
 * 最近记录单行截断；低频操作（开始冬眠/确认出眠/补录冬眠/编辑）收进「⋯」菜单——
 * 动作执行即收（menuAction 包装）、点卡片外即收（document click 监听 + 卡片 contains
 * 自身守卫，dots 不拦冒泡，跨卡点 dots 时旧卡菜单即收=关旧开新）；
 * 菜单容器 v-show 保 DOM，按钮保留原类名
 * （edit-btn/hib-btn/wake-btn/past-btn），既有测试直接点这些按钮不受影响。
 * 冬眠横幅瘦成一条，「改期」入口保留。记账/冬眠成功抛 saved 让外层 refresh（数据驱动重算）。
 * 票 02：撤食块（follow）走三态——无待撤置灰禁点 / 待撤可点 / 逾期红，点击开通用
 * 打卡面板（QuickLogDialog）走 log_care 闭环；派生态随数据刷新自动重算。
 * 窝头像票 02：头部左侧头像（票 01 的 Colony.avatar 投影载荷），方形裁剪框按
 * crop 坐标定位 + 形状遮罩（默认圆形，全局偏好在票 03 接线）；点击 = 打开该窝
 * 巢况时间线（与「巢况」按钮同一弹窗）。取图双通路：桌面 asset 协议
 * （photoSrc + getPhotoAbsDir），网页 loadPhotoBlobUrl blob（凭证不进 URL，
 * 换头像/卸载释放）；加载失败回退 🐜 占位，不崩溃。
 * 窝头像票 03：形状接全局偏好——shape prop 缺省时跟随 ipc.ts 的全局镜像
 * （首个挂载的卡片经 get_avatar_shape 拉一次，多实例共享；设置页保存成功后
 * 镜像更新，已挂载卡片经响应性立即切换），显式传入的 prop 仍优先（票 02 契约）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Colony, ColonyAction, PhotoCrop } from "../types";
import {
  actionTile,
  feedingTooltip,
  formatRecent,
  isFeeding,
  isRetrieval,
  retrievalStateOf,
  retrievalTile,
  type TileView,
} from "../lib/care";
import {
  avatarShape,
  claimAvatarShapePrefLoad,
  getAvatarShape,
  getPhotoAbsDir,
  isTauri,
  normalizeAvatarShape,
  saveAvatarShapePref,
} from "../lib/ipc";
import { loadPhotoBlobUrl, photoSrc, revokeObjectUrl } from "../lib/photos";
import { checkinCardLine } from "../lib/checkin";
import { hibernationBanner } from "../lib/hibernation";
import { todayIso } from "../lib/dates";
import FeedDialog from "./FeedDialog.vue";
import HibernationDialog from "./HibernationDialog.vue";
import NestCheckinDialog from "./NestCheckinDialog.vue";
import QuickLogDialog from "./QuickLogDialog.vue";

const props = withDefaults(
  defineProps<{
    colony: Colony;
    /** 头像显示形状（窝头像票 02）：circle=圆形遮罩 / square=方形；缺省 =
     *  跟随全局偏好（窝头像票 03），显式传入优先于全局（票 02 契约不破）。 */
    shape?: "circle" | "square";
  }>(),
  { shape: undefined },
);
const emit = defineEmits<{ edit: []; saved: [] }>();

/** 实际渲染形状（窝头像票 03）：显式 prop 优先，否则跟随全局偏好镜像。 */
const shapeClass = computed(() => props.shape ?? avatarShape.value);

const STATUS_TEXT: Record<Colony["status"], string> = {
  active: "● 活跃",
  hibernating: "❄ 冬眠中",
  ended: "◻ 已结束",
};

const hibernating = computed(() => props.colony.status === "hibernating");

/** 冬眠横幅数据（有开放段才显示；角标阈值 7 天 = spec 默认，纯展示，通知属票 06）。 */
const banner = computed(() =>
  hibernating.value && props.colony.hibernation !== null
    ? hibernationBanner(props.colony.hibernation, todayIso())
    : null,
);

// 撤食块（票 02）走三态视图：无待撤置灰禁点 / 待撤正常可点 / 逾期红，无倒计时；
// 冬眠不静音（发霉不等人）。其余操作维持原 actionTile（含冬眠静音）。
const tiles = computed(() =>
  props.colony.actions.map((a) => ({
    action: a,
    view: (isRetrieval(a) ? retrievalTile(a) : actionTile(a, hibernating.value)) as TileView,
    disabled: isRetrieval(a) && retrievalStateOf(a) === "none",
  })),
);

const recentLine = computed(() => formatRecent(props.colony.recent));

/** 巢况摘要行（webui-checkin 票 02）：最新一组数 + 距上次登记天数；从未登记为空串（隐藏）。 */
const checkinLine = computed(() => checkinCardLine(props.colony.checkin));

// ── 窝头像（窝头像票 02）──────────────────────────────────────
// 头像引用 = 票 01 投影载荷 Colony.avatar（前端不重复推导）。跟随按 rel_path：
// 首页刷新会重建 colony 对象，同一张照片不重取不重挂；裁剪改了（rel 不变）由
// 下面的样式 computed 直接响应。取图双通路沿 NestCheckinDialog 先例：桌面
// asset 协议（getPhotoAbsDir 一次 + photoSrc），网页 loadPhotoBlobUrl blob
// （凭证不进 URL）；换头像与卸载经 revokeObjectUrl 释放，失败回退 🐜 占位。
const avatarUrl = ref("");
const avatarBroken = ref(false);
/** 图片 natural 尺寸（load 后可得；裁剪定位的输入，happy-dom/真实浏览器一致） */
const avatarImgSize = ref<{ w: number; h: number } | null>(null);
const avatarRel = computed(() => props.colony.avatar?.rel_path ?? null);
/** 竞态守卫：连换头像/卸载时只让最后一轮取图落地 */
let avatarSeq = 0;

const photoAbsDir = ref("");

async function loadAvatar(rel: string | null): Promise<void> {
  const seq = ++avatarSeq;
  if (avatarUrl.value !== "") {
    revokeObjectUrl(avatarUrl.value);
    avatarUrl.value = "";
  }
  avatarBroken.value = false;
  avatarImgSize.value = null;
  if (rel === null) return;
  if (isTauri()) {
    if (photoAbsDir.value === "") {
      try {
        photoAbsDir.value = await getPhotoAbsDir();
      } catch {
        // 根目录拿不到：占位兜底（下次头像变化再试）
        if (seq === avatarSeq) avatarBroken.value = true;
        return;
      }
    }
    if (seq !== avatarSeq) return;
    const src = photoSrc(rel, photoAbsDir.value);
    if (src === "") {
      avatarBroken.value = true;
      return;
    }
    avatarUrl.value = src;
    return;
  }
  try {
    const url = await loadPhotoBlobUrl(rel);
    if (seq !== avatarSeq) {
      revokeObjectUrl(url); // 期间头像已换/组件已卸载：立即释放
      return;
    }
    avatarUrl.value = url;
  } catch {
    if (seq === avatarSeq) avatarBroken.value = true;
  }
}

watch(
  avatarRel,
  (rel) => {
    void loadAvatar(rel);
  },
  { immediate: true },
);

/**
 * 裁剪定位样式（规格：方形裁剪框按裁剪坐标缩放定位，形状只是遮罩）。
 * crop 语义：size = 边长 / 图片长边，x/y = 裁剪左上角各按宽/高归一化——与
 * 「整图 cover 进方框取中心最大方形」的默认公式自洽（size = 短边/长边、x/y
 * 使区域居中）。渲染：img 绝对定位铺满 sidePx 的放大倍数，再负偏移裁剪原点。
 */
const avatarImgStyle = computed<Record<string, string>>(() => {
  const nat = avatarImgSize.value;
  const style: Record<string, string> = {};
  if (nat === null) return style;
  const crop: PhotoCrop =
    props.colony.avatar?.crop ?? {
      // 默认居中：中心最大方形
      size: Math.min(nat.w, nat.h) / Math.max(nat.w, nat.h),
      x: Math.max(0, 1 - Math.min(nat.w, nat.h) / nat.w) / 2,
      y: Math.max(0, 1 - Math.min(nat.w, nat.h) / nat.h) / 2,
    };
  const sidePx = crop.size * Math.max(nat.w, nat.h);
  if (!(sidePx > 0)) return style;
  style.width = `${(nat.w / sidePx) * 100}%`;
  style.height = `${(nat.h / sidePx) * 100}%`;
  style.left = `${((-crop.x * nat.w) / sidePx) * 100}%`;
  style.top = `${((-crop.y * nat.h) / sidePx) * 100}%`;
  return style;
});

function onAvatarLoad(e: Event): void {
  const el = e.target as HTMLImageElement;
  if (el.naturalWidth > 0 && el.naturalHeight > 0) {
    avatarImgSize.value = { w: el.naturalWidth, h: el.naturalHeight };
  }
}

/** 文件缺失/解码失败（img error 事件）：占位兜底，不显示破图 */
function onAvatarError(): void {
  avatarBroken.value = true;
}

// ── 「⋯」菜单（交互第三轮 #7）：开合 + 两路收起 ──
const menuOpen = ref(false);
const cardRef = ref<HTMLElement | null>(null);

function toggleMenu() {
  menuOpen.value = !menuOpen.value;
}

/** 菜单动作执行即收（复审 #12）；点卡片外也收（document click，含点别卡 dots 的跨卡场景）。 */
function menuAction(fn: () => void) {
  menuOpen.value = false;
  fn();
}

/** 点自身卡内（含 dots）不收——dots 靠 toggle 开合；点卡外（别卡/空白处）即收。 */
function onDocClick(e: MouseEvent) {
  if (!menuOpen.value) return;
  const root = cardRef.value;
  if (root !== null && e.target instanceof Node && root.contains(e.target)) return;
  menuOpen.value = false;
}

onMounted(() => {
  document.addEventListener("click", onDocClick);
  // 全局形状偏好启动读取（窝头像票 03）：首个挂载的卡片拉一次，后续卡片共享
  // （含失败，不逐卡重试）；读不出保默认圆形——外观偏好失败不惊动、不挡头像渲染
  if (claimAvatarShapePrefLoad()) return;
  void (async () => {
    try {
      saveAvatarShapePref(normalizeAvatarShape(await getAvatarShape()));
    } catch {
      /* 保持默认 circle；镜像未被污染，冷启动再试 */
    }
  })();
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
  // 头像收尾：在途取图作废 + 释放网页端 blob（桌面 asset URL 会被放过）
  avatarSeq += 1;
  revokeObjectUrl(avatarUrl.value);
});

const showFeed = ref(false);
const feedAction = ref<ColonyAction | null>(null);

const showQuick = ref(false);
const quickAction = ref<ColonyAction | null>(null);

const showHibernation = ref(false);
const hibernationMode = ref<"start" | "wake" | "past" | "edit">("start");

const showCheckin = ref(false);

function openHibernation(mode: "start" | "wake" | "past" | "edit") {
  hibernationMode.value = mode;
  showHibernation.value = true;
}

function onHibernationSaved() {
  showHibernation.value = false;
  emit("saved");
}

function onTile(a: ColonyAction) {
  if (isFeeding(a)) {
    feedAction.value = a;
    showFeed.value = true;
    return;
  }
  // 撤食块无待撤不响应（disabled 按钮本就不触发，这里兜底）
  if (isRetrieval(a) && retrievalStateOf(a) === "none") return;
  quickAction.value = a;
  showQuick.value = true;
}

function onQuickSaved() {
  showQuick.value = false;
  quickAction.value = null;
  emit("saved");
}

function onFeedSaved() {
  showFeed.value = false;
  feedAction.value = null;
  emit("saved");
}

function onCheckinSaved() {
  showCheckin.value = false;
  emit("saved");
}
</script>

<template>
  <article ref="cardRef" class="card" :class="{ hib: hibernating }" :data-colony-id="colony.id">
    <div class="chead">
      <!-- 窝头像（窝头像票 02）：点击 = 打开巢况时间线，与「巢况」按钮同一弹窗 -->
      <button
        class="avatar"
        :class="shapeClass"
        type="button"
        data-testid="colony-avatar"
        title="查看巢况时间线"
        aria-label="查看该窝的巢况时间线"
        @click="showCheckin = true"
      >
        <img
          v-if="avatarUrl !== '' && !avatarBroken"
          class="avatar-img"
          :class="{ loaded: avatarImgSize !== null }"
          :src="avatarUrl"
          alt=""
          :style="avatarImgStyle"
          @load="onAvatarLoad"
          @error="onAvatarError"
        />
        <span v-else class="avatar-ph" data-testid="avatar-ph">🐜</span>
      </button>
      <span class="cname">{{ colony.name }}</span>
      <span v-if="colony.species" class="chip sp">{{ colony.species }}</span>
      <span class="chip st" :class="{ hib: colony.status === 'hibernating' }">
        {{ STATUS_TEXT[colony.status] }}
      </span>
      <span class="daysbox"><span class="n">{{ colony.days_raised }}</span> <span class="l">天</span></span>
    </div>

    <div v-if="banner" class="banner" data-testid="hib-banner">
      {{ banner.line }}
      <span v-if="banner.nearWake" class="chip wake">临近出眠</span>
      <button
        class="resched-btn"
        type="button"
        title="修改预计出眠日：未发的临近/出眠提醒按新日期重算"
        @click="openHibernation('edit')"
      >
        改期
      </button>
    </div>

    <div class="tiles">
      <button
        v-for="{ action: a, view, disabled } in tiles"
        :key="a.action_id"
        class="tile"
        :class="view.tone"
        :data-action-id="a.action_id"
        type="button"
        :disabled="disabled"
        :title="a.is_feeding && a.foods.length > 0 ? feedingTooltip(a.foods) : undefined"
        @click="onTile(a)"
      >
        <span class="t-top">
          <span v-if="a.icon" class="t-ico">{{ a.icon }}</span>
          <span class="t-name">{{ a.name }}</span>
          <span v-if="a.kind === 'log_only'" class="t-tag">仅登记</span>
        </span>
        <span class="pill">{{ view.text }}</span>
      </button>
    </div>

    <div class="foot">
      <span class="recent">{{ recentLine }}</span>
      <button class="dots" type="button" title="编辑 / 冬眠等更多操作" @click="toggleMenu">⋯</button>
    </div>

    <div v-if="checkinLine" class="checkin-line" data-testid="checkin-line">{{ checkinLine }}</div>

    <div class="card-actions">
      <button class="checkin-btn" type="button" title="蚁口 / 换巢 / 备注的时间线" @click="showCheckin = true">
        巢况
      </button>
    </div>

    <!-- 交互第三轮 #7：低频操作收进 ⋯ 菜单（v-show 保 DOM，按钮原类名与测试兼容） -->
    <div v-show="menuOpen" class="card-menu" @click.stop>
      <button
        v-if="colony.status === 'active'"
        class="m-item hib-btn"
        type="button"
        title="期间提醒静音、仍可记账"
        @click="menuAction(() => openHibernation('start'))"
      >
        ❄ 开始冬眠
      </button>
      <button
        v-if="hibernating"
        class="m-item wake-btn"
        type="button"
        @click="menuAction(() => openHibernation('wake'))"
      >
        ☀ 确认出眠
      </button>
      <button
        v-if="colony.status !== 'ended'"
        class="m-item past-btn"
        type="button"
        title="补录已闭合的过去冬眠段"
        @click="menuAction(() => openHibernation('past'))"
      >
        📅 补录冬眠
      </button>
      <!-- 终局评审：窝的编辑是桌面专属（网页端 API 白名单挡住 update_colony 等），浏览器不渲染入口 -->
      <button v-if="isTauri()" class="m-item edit-btn" type="button" @click="menuAction(() => emit('edit'))">✏️ 编辑窝信息</button>
    </div>

    <FeedDialog
      v-if="showFeed && feedAction !== null"
      :colony="colony"
      :action="feedAction"
      @close="showFeed = false"
      @saved="onFeedSaved"
    />
    <QuickLogDialog
      v-if="showQuick && quickAction !== null"
      :colony="colony"
      :action="quickAction"
      @close="showQuick = false"
      @saved="onQuickSaved"
    />
    <HibernationDialog
      v-if="showHibernation"
      :colony="colony"
      :mode="hibernationMode"
      @close="showHibernation = false"
      @saved="onHibernationSaved"
    />
    <NestCheckinDialog
      v-if="showCheckin"
      :colony="colony"
      @close="showCheckin = false"
      @saved="onCheckinSaved"
    />
  </article>
</template>

<style scoped>
.card {
  position: relative;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 10px 12px;
  box-shadow: var(--shadow);
}

/* 冬眠整卡灰化（顶部向下渐隐的冷灰） */
.card.hib {
  background: linear-gradient(180deg, var(--hib-soft), var(--card) 60%);
}

.chead {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap; /* 超长窝名/物种折行不截断（mock-d 治理，随变体 A 一并落地） */
}

/* ── 窝头像（窝头像票 02）：方形裁剪框 + 形状遮罩（默认圆形），点击开巢况时间线 ── */
.avatar {
  position: relative;
  flex: none;
  width: 40px;
  height: 40px;
  padding: 0;
  border: 1px solid var(--border);
  background: var(--tile);
  overflow: hidden;
  cursor: pointer;
  display: grid;
  place-items: center;
  border-radius: 50%; /* 默认形状 circle：圆形遮罩挖角，裁剪数据不受影响 */
}

.avatar.square {
  border-radius: 8px;
}

.avatar:hover {
  border-color: var(--accent);
}

/* 裁剪定位 img：绝对定位 + 百分比宽高/偏移（style 由 avatarImgStyle 计算）；
   载入拿到 natural 尺寸前不显示，避免闪一帧未缩放的原图 */
.avatar-img {
  position: absolute;
  max-width: none; /* 放大裁剪超出方框属预期，不受全局 img 宽度约束 */
  opacity: 0;
  transition: opacity 0.15s;
}

.avatar-img.loaded {
  opacity: 1;
}

.avatar-ph {
  font-size: 22px;
  line-height: 1;
}

.cname {
  font-size: 15px;
  font-weight: 700;
  white-space: nowrap;
}

.chip {
  font-size: 11px;
  padding: 0 8px;
  border-radius: 999px;
  border: 1px solid transparent;
  white-space: nowrap;
}

.chip.sp {
  background: var(--accent-soft);
  color: var(--accent-deep);
}

.chip.st {
  background: var(--ok-soft);
  color: var(--ok);
}

.chip.st.hib {
  background: var(--hib-soft);
  color: var(--hib);
}

.daysbox {
  margin-left: auto;
  white-space: nowrap;
}

.daysbox .n {
  font-size: 16px;
  font-weight: 800;
}

.daysbox .l {
  font-size: 10px;
  color: var(--muted);
}

/* ── 冬眠横幅：瘦成一条 ── */
.banner {
  margin-top: 8px;
  padding: 4px 10px;
  border-radius: 8px;
  background: var(--hib-soft);
  color: var(--hib);
  font-size: 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.chip.wake {
  background: var(--accent-soft);
  color: var(--accent-deep);
  font-weight: 600;
}

/* 横幅上的「改期」入口（票 09 停靠 D，改期入口保留） */
.resched-btn {
  margin-left: auto;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--hib);
  font: inherit;
  font-size: 11px;
  padding: 0 8px;
  border-radius: 999px;
  cursor: pointer;
  white-space: nowrap;
}

.resched-btn:hover {
  border-color: var(--hib);
  color: var(--text);
}

/* ── 操作块：两行格（mock-d 变体 A）——上行名字/标签，下行状态。
   单行两列在 ~117px 格宽装不下 0.5.0「距上次 N 天 / 周期 M 天」，实测溢出；
   两行结构 + 药丸自然折行，零截断。2 列网格（mock C1）维持。 */
.tiles {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
  margin-top: 8px;
}

.tile {
  border: 1px solid var(--border);
  background: var(--tile);
  border-radius: 9px;
  padding: 5px 9px;
  cursor: pointer;
  font: inherit;
  color: var(--text);
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 13px;
  transition: 0.12s;
  min-width: 0;
}

.tile:hover {
  border-color: var(--accent);
}

/* 上行：图标 + 名字 + 仅登记标签；挤不下时标签折到名下，不截断 */
.t-top {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex-wrap: wrap;
}

.tile .t-ico {
  font-size: 14px;
}

.tile .t-name {
  font-weight: 600;
  white-space: nowrap;
}

.t-tag {
  font-size: 10px;
  font-weight: 400;
  color: var(--muted);
  border: 1px solid var(--border);
  padding: 0 5px;
  border-radius: 999px;
  white-space: nowrap;
}

/* 下行状态药丸：左对齐独立一行；长清单（该喂X、Y了）自然折行 */
.pill {
  align-self: flex-start;
  max-width: 100%;
  font-size: 11px;
  padding: 0 8px;
  border-radius: 999px;
  white-space: normal;
}

.tile.reg .pill,
.tile.none .pill {
  background: var(--tile);
  color: var(--muted);
}

.tile.ok .pill {
  background: var(--ok-soft);
  color: var(--ok);
}

.tile.bad {
  border-color: var(--bad);
  background: var(--bad-soft);
}

.tile.bad .pill {
  background: #fff;
  color: var(--bad);
  font-weight: 600;
}

.tile.mute {
  opacity: 0.55;
  cursor: pointer;
}

.tile.mute .pill {
  background: var(--hib-soft);
  color: var(--hib);
}

.tile:disabled {
  opacity: 0.6;
  cursor: default;
}

/* ── 底部行（最近记录截断一行）+「⋯」菜单 ── */
.foot {
  margin-top: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.recent {
  flex: 1;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 巢况摘要行（webui-checkin 票 02）：与最近记录行同字号，紧随其后 */
.checkin-line {
  margin-top: 6px;
  font-size: 12px;
  color: var(--muted);
}

.card-actions {
  margin-top: 10px;
  display: flex;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 6px;
}

/* 巢况按钮沿用卡片按钮基样式（其余低频按钮已收进 ⋯ 菜单，样式随菜单） */
.checkin-btn {
  border: 1px solid var(--border-strong);
  background: var(--card);
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  padding: 2px 12px;
  border-radius: 8px;
  cursor: pointer;
}

.dots {
  flex: none;
  border: none;
  background: transparent;
  color: var(--muted);
  font-size: 16px;
  cursor: pointer;
  padding: 0 4px;
  border-radius: 6px;
  line-height: 1;
}

.checkin-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.dots:hover {
  background: var(--tile);
  color: var(--text);
}

.card-menu {
  position: absolute;
  right: 10px;
  bottom: 34px;
  background: var(--card);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  box-shadow: 0 8px 30px rgba(60, 50, 30, 0.15);
  padding: 4px;
  z-index: 20;
  min-width: 108px;
}

.m-item {
  display: block;
  width: 100%;
  border: none;
  background: transparent;
  font: inherit;
  font-size: 12px;
  color: var(--text);
  padding: 6px 10px;
  border-radius: 7px;
  cursor: pointer;
  text-align: left;
  white-space: nowrap;
}

.m-item:hover {
  background: var(--accent-soft);
  color: var(--accent-deep);
}
</style>
