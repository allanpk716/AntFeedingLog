/**
 * 全局「今天」时钟源：顶栏日期、统计页时间范围、首页卡片距上次/红标等一切
 * 依赖今天的展示，从本模块的响应式 today 取值、或由它的跨天变更驱动重拉——
 * 应用开着跨过 00:00 后零操作自动追上真实日期。
 *
 * 刷新策略（共识 2026-09-22）：每分钟一次轻量 tick ＋ 窗口 focus /
 * visibilitychange 兜底（治两类错过零点：系统睡眠唤醒、浏览器后台标签页
 * 定时器降频）。日期不变不写 ref，不触发任何下游重渲染。时钟由 App 根
 * 挂载启动（常驻不卸载，桌面端与网页端同一前端同享）；dates.ts 是纯函数、
 * 这里是带副作用的单例，两者分工不混。
 */
import { readonly, ref, type Ref } from "vue";
import { todayIso } from "./dates";

const today = ref(todayIso());
const frozen = readonly(today);

/** 立即对表：真实日期变了才写 ref（同日静默）。 */
export function syncToday(): void {
  const iso = todayIso();
  if (iso !== today.value) today.value = iso;
}

function onWakeSignal() {
  syncToday();
}

let timer: ReturnType<typeof setInterval> | null = null;

/** 幂等启动时钟：先对表一次，再挂分钟 tick 与兜底监听。 */
export function startTodayClock(): void {
  if (timer !== null) return;
  syncToday();
  timer = setInterval(syncToday, 60_000);
  window.addEventListener("focus", onWakeSignal);
  document.addEventListener("visibilitychange", onWakeSignal);
}

/** 停表清监听（App 根卸载时调用；测试隔离用）。幂等。 */
export function stopTodayClock(): void {
  if (timer === null) return;
  clearInterval(timer);
  timer = null;
  window.removeEventListener("focus", onWakeSignal);
  document.removeEventListener("visibilitychange", onWakeSignal);
}

/** 全局响应式今天（YYYY-MM-DD，只读）。 */
export function todayIsoRef(): Readonly<Ref<string>> {
  return frozen;
}
