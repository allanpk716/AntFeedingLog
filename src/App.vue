<script setup lang="ts">
/**
 * 首页：按地点分组渲染窝卡片（分组顺序 = 地点清单顺序，空地点「未分组」最后），
 * 已结束的窝收底部折叠区（默认折叠）。「＋ 新建窝」在顶栏（交互第三轮 #7），与设置入口同排。
 * 卡片操作块/喂食弹窗在 ColonyCard 内（票 03）：记账成功抛 saved → refresh 数据驱动重算。
 * 设置弹窗（票 04）：字典管理三 tab，任何变更抛 changed → refresh，卡片红/灰即时跟上。
 * 今天时钟源在根启动（常驻不卸载）：顶栏日期从响应式 today 取值，跨天变更
 * 驱动 refresh 重拉——卡片距上次/红标/饲养天数跟着换天，零操作自动追上。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getWebUiWizardDone, isTauri, listColonies, listLocations, subscribe } from "./lib/ipc";
import { watchDataVersion } from "./lib/versionSync";
import type { Colony, LocationItem } from "./types";
import { groupColonies, splitColonies } from "./lib/home";
import { todayLabel } from "./lib/dates";
import { startTodayClock, stopTodayClock, todayIsoRef } from "./lib/today";
import ColonyCard from "./components/ColonyCard.vue";
import ColonyFormDialog from "./components/ColonyFormDialog.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import StatsPage from "./components/StatsPage.vue";
import LogListPage from "./components/LogListPage.vue";
import ToastHost from "./components/ToastHost.vue";
import WebUiWizard from "./components/WebUiWizard.vue";

/** 顶栏三页 nav（票 08 接活「记录」） */
type Page = "home" | "stats" | "logs";
const page = ref<Page>("home");

const colonies = ref<Colony[]>([]);
const locations = ref<LocationItem[]>([]);
const pageError = ref("");

const showForm = ref(false);
const editing = ref<Colony | null>(null);
const showSettings = ref(false);
const endedOpen = ref(false);
/** 网页端首启向导（webui-checkin 票 03）：启动时查到「未做」才弹一次。 */
const showWebUiWizard = ref(false);

const activeColonies = computed(() => splitColonies(colonies.value).active);
const endedColonies = computed(() => splitColonies(colonies.value).ended);
const groups = computed(() => groupColonies(activeColonies.value, locations.value));

/** 顶栏日期从全局今天源派生（`T00:00` 本地零点解析：任何时区下星期都正确）。 */
const today = todayIsoRef();
const todayText = computed(() => todayLabel(new Date(`${today.value}T00:00`)));

// 跨天 → 重拉首页数据（距上次/红标/饲养天数跟新的一天走）
watch(today, () => void refresh());

async function refresh() {
  try {
    const [cols, locs] = await Promise.all([
      listColonies(),
      listLocations(),
    ]);
    colonies.value = cols;
    locations.value = locs;
    pageError.value = "";
  } catch (e) {
    pageError.value = String(e);
  }
}

function openCreate() {
  editing.value = null;
  showForm.value = true;
}

function openEdit(colony: Colony) {
  editing.value = colony;
  showForm.value = true;
}

function onSaved() {
  showForm.value = false;
  void refresh();
}

function onSettingsChanged() {
  void refresh();
}

function onSettingsClosed() {
  showSettings.value = false;
  void refresh();
}

onMounted(() => {
  // 今天时钟源在根启动（分钟 tick + focus/visibilitychange 兜底）；根卸载才停表
  startTodayClock();
  void refresh();
  // 恢复完成广播（数据安全二期票 04，语义=无条件刷新，票 06 不改）：整库被
  // 替换，各页数据全部重拉——首页在此刷新；统计/记录页离开再进时按 v-if
  // 重挂载自然重拉
  void subscribe("db-restored", () => {
    void refresh();
  });
  // 数据版本广播（webui-checkin 票 06）：任一端记录、开着的一端自动刷新——
  // 同 epoch 版本落后重拉当前页；epoch 变（电脑重启过）无条件重拉。桌面走
  // data-version 事件、浏览器走 SSE（versionSync 对账）；恢复完成在浏览器侧
  // 也经恢复后的版本帧到达（服务端已做跨恢复单调抬升）。本组件是根，常驻
  // 不卸载，与 db-restored 同款不退订。
  void watchDataVersion(() => {
    void refresh();
  });
  // 网页端首启向导（webui-checkin 票 03）：只在明确查到「未做」（false）时弹；
  // 查询失败（网页端浏览器态/异常）静默——向导不该挡住正常使用
  void getWebUiWizardDone()
    .then((done) => {
      if (done === false) showWebUiWizard.value = true;
    })
    .catch(() => {});
});

onBeforeUnmount(() => stopTodayClock());
</script>

<template>
  <div class="page">
    <header class="topbar">
      <div class="brand">🐜 蚂蚁饲养日志</div>
      <nav class="nav">
        <button
          class="tab"
          :class="{ active: page === 'home' }"
          type="button"
          @click="page = 'home'"
        >
          首页
        </button>
        <button
          v-if="isTauri()"
          class="tab"
          :class="{ active: page === 'stats' }"
          type="button"
          @click="page = 'stats'"
        >
          统计
        </button>
        <button
          class="tab"
          :class="{ active: page === 'logs' }"
          type="button"
          @click="page = 'logs'"
        >
          记录
        </button>
      </nav>
      <div class="today">{{ todayText }}</div>
      <div class="tools">
        <!-- 终局评审：新建窝/设置是桌面专属（网页端 API 白名单本就挡住），浏览器不渲染入口 -->
        <button v-if="isTauri()" class="ghost-btn new-top-btn" type="button" @click="openCreate">＋ 新建窝</button>
        <button v-if="isTauri()" class="ghost-btn settings-btn" type="button" title="字典管理（操作 / 食物 / 地点）" @click="showSettings = true">
          ⚙ 设置
        </button>
      </div>
    </header>

    <!-- 交互第三轮 #6：顶栏固定，内容区独立滚动 -->
    <div class="page-body">
      <main v-if="page === 'home'" class="container">
      <p v-if="pageError" class="page-error">{{ pageError }}</p>
      <p v-if="colonies.length === 0" class="empty">暂无窝</p>

      <section v-for="g in groups" :key="g.key" class="group">
        <div class="group-head">
          <h2 class="group-title">{{ g.title }}</h2>
          <span class="cnt">{{ g.colonies.length }} 窝</span>
          <div class="rule"></div>
        </div>
        <div class="cards vp-cards">
          <ColonyCard
            v-for="c in g.colonies"
            :key="c.id"
            :colony="c"
            @edit="openEdit(c)"
            @saved="void refresh()"
          />
        </div>
      </section>

      <section v-if="endedColonies.length > 0" class="ended-area">
        <button class="ended-toggle" type="button" @click="endedOpen = !endedOpen">
          已结束（{{ endedColonies.length }}）{{ endedOpen ? "▲ 收起" : "▼ 展开" }}
        </button>
        <div v-show="endedOpen" class="ended-section">
          <div class="cards vp-cards">
            <ColonyCard
              v-for="c in endedColonies"
              :key="c.id"
              :colony="c"
              @edit="openEdit(c)"
              @saved="void refresh()"
            />
          </div>
        </div>
      </section>
    </main>

    <StatsPage v-if="page === 'stats'" />

    <!-- 记录流水（票 08）：任何编辑/删除抛 changed → refresh，首页红绿态即时重算 -->
    <LogListPage v-if="page === 'logs'" @changed="void refresh()" />
    </div>

    <ColonyFormDialog
      v-if="showForm"
      :editing="editing"
      :colonies="colonies"
      :locations="locations"
      @close="showForm = false"
      @saved="onSaved"
    />
    <SettingsDialog
      v-if="showSettings"
      @close="onSettingsClosed"
      @changed="onSettingsChanged"
    />
    <WebUiWizard v-if="showWebUiWizard" @close="showWebUiWizard = false" />
    <!-- 轻提示宿主（保湿方式+轻提示票 03）：全局唯一出口挂应用根，桌面端与网页端同一前端同享 -->
    <ToastHost />
  </div>
</template>

<style scoped>
.page {
  /* 视觉基线 mocks/mock-a-light.html */
  --bg: #f6f4f1;
  --card: #ffffff;
  --tile: #faf8f5;
  --text: #2c2822;
  --muted: #8f887d;
  --border: #e8e2d8;
  --border-strong: #d8d1c4;
  --accent: #d97706;
  --accent-deep: #b45309;
  --accent-soft: #fdf1de;
  --ok: #188a4b;
  --ok-soft: #e7f5ec;
  --bad: #d13d3d;
  --bad-soft: #fcebeb;
  --hib: #5b6472;
  --hib-soft: #eef0f3;
  --shadow: 0 1px 2px rgba(60, 50, 30, 0.05), 0 4px 14px rgba(60, 50, 30, 0.06);
  --overlay: rgba(40, 35, 25, 0.35);

  height: 100vh;          /* 交互第三轮 #6：外壳固定，不再整页滚 */
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  font-size: 14px;
  line-height: 1.55;
}

.topbar {
  flex: none;             /* 顶栏固定高度，不参与滚动 */
  background: var(--card);
  border-bottom: 1px solid var(--border);
}

/* 唯一滚动容器：切换页面只有这里滚，顶栏常驻 */
.page-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.hbar,
.topbar {
  padding: 10px 20px;
  display: flex;
  align-items: center;
  gap: 18px;
}

.brand {
  font-size: 17px;
  font-weight: 700;
  white-space: nowrap;
}

/* 三页 nav（视觉基线 mocks/mock-b-stats.html 的 .nav/.tab） */
.nav {
  display: flex;
  gap: 4px;
}

.tab {
  padding: 5px 14px;
  border: none;
  border-radius: 999px;
  cursor: pointer;
  color: var(--muted);
  background: transparent;
  font: inherit;
  font-size: 14px;
}

.tab.active {
  background: var(--accent-soft);
  color: var(--accent-deep);
  font-weight: 600;
}

.tab:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

/* 顶栏今天日期（视觉基线 mocks/mock-a-light.html 的 .today） */
.today {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.tools {
  margin-left: auto;
  display: flex;
  gap: 8px;
}

.ghost-btn {
  padding: 5px 12px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  cursor: pointer;
  color: var(--text);
  font-size: 13px;
  background: var(--card);
  font-family: inherit;
}

.ghost-btn:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.container {
  max-width: 1080px;
  margin: 0 auto;
  padding: 0 20px 32px;   /* 原 80px 底衬是常驻滚动条元凶之一 */
}

.empty {
  margin-top: 60px;
  text-align: center;
  font-size: 18px;
  color: var(--muted);
}

.page-error {
  margin-top: 16px;
  padding: 8px 12px;
  border-radius: 10px;
  background: var(--bad-soft);
  color: var(--bad);
  font-size: 13px;
}

/* 交互第三轮 #7：分组边距收紧（视觉基线 mock-c-home-cards），首个分组贴顶 */
.group {
  margin-top: 14px;
}

.group:first-child {
  margin-top: 0;
}

.group-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}

.group-head h2 {
  font-size: 14px;
}

.group-head .cnt {
  font-size: 12px;
  color: var(--muted);
  background: var(--tile);
  border: 1px solid var(--border);
  padding: 1px 9px;
  border-radius: 999px;
}

.group-head .rule {
  flex: 1;
  height: 1px;
  background: var(--border);
}

.cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 10px;
}

.ended-area {
  margin-top: 30px;
}

.ended-toggle {
  border: 1px solid var(--border-strong);
  background: var(--tile);
  color: var(--muted);
  border-radius: 10px;
  font: inherit;
  font-size: 13px;
  padding: 6px 14px;
  cursor: pointer;
}

.ended-toggle:hover {
  border-color: var(--accent);
  color: var(--accent-deep);
}

.ended-section {
  margin-top: 12px;
}

/* ── 手机竖屏（webui-checkin 票 08）：≤480px 单列卡片 + 顶栏可换行；
   vp-cards 是媒体查询落点（断点类名供组件测试断言——jsdom 不套用媒体查询）── */
@media (max-width: 480px) {
  .topbar {
    flex-wrap: wrap;
    gap: 8px;
    padding: 10px 12px;
  }

  .container {
    padding: 0 12px 60px;
  }

  .vp-cards {
    grid-template-columns: 1fr;
  }
}
</style>
