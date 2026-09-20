import { readonly, ref } from "vue";

/**
 * 全局唯一轻提示 store（保湿方式+轻提示票 03）：展示端是挂应用根的
 * ToastHost.vue，桌面端与网页端同一前端同享。判定原则见项目根 CLAUDE.md
 * 「操作反馈规范」：操作完成后界面没有天然反馈的写操作必须弹；字段级校验
 * 错误维持内联红字不进轻提示；催办提醒走系统通知渠道，永不进轻提示。
 * 各调用点只调用本模块 API，不得自造提示样式。
 */

export type ToastKind = "success" | "error";

export interface ToastItem {
  id: number;
  kind: ToastKind;
  /** 主文案：如「已保存」「备份失败」 */
  message: string;
  /** 失败原因（失败轻提示文案带原因），成功档恒为空串 */
  reason: string;
}

/** 成功绿色约 2.5 秒自动消失；失败红色约 5 秒（CLAUDE.md「操作反馈规范」） */
const SUCCESS_DURATION_MS = 2500;
const ERROR_DURATION_MS = 5000;
/** 同屏至多 3 条堆叠，超限更早先走 */
const MAX_VISIBLE = 3;

const toasts = ref<ToastItem[]>([]);
let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function push(kind: ToastKind, message: string, reason = ""): void {
  const id = nextId++;
  toasts.value.push({ id, kind, message, reason });
  // 超上限：最旧的先走（dismissToast 会顺带清掉它的挂起计时器）
  while (toasts.value.length > MAX_VISIBLE) {
    dismissToast(toasts.value[0]!.id);
  }
  timers.set(
    id,
    setTimeout(() => dismissToast(id), kind === "success" ? SUCCESS_DURATION_MS : ERROR_DURATION_MS),
  );
}

/** 成功轻提示（绿色，约 2.5 秒自动消失） */
export function showSuccess(message: string): void {
  push("success", message);
}

/** 失败轻提示（红色，约 5 秒；文案带原因，可点 × 提前关） */
export function showError(message: string, reason = ""): void {
  push("error", message, reason);
}

/** 提前关（× 按钮）；自动消失到点也走这里 */
export function dismissToast(id: number): void {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
  toasts.value = toasts.value.filter((item) => item.id !== id);
}

/** 只读视图：ToastHost 渲染用 */
export const toastItems = readonly(toasts);

/** 清空全部并撤销挂起计时（测试复位用；运行时不接） */
export function clearToasts(): void {
  for (const timer of timers.values()) {
    clearTimeout(timer);
  }
  timers.clear();
  toasts.value = [];
}
