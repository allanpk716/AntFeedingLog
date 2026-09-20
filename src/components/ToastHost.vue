<script setup lang="ts">
/**
 * 轻提示宿主（保湿方式+轻提示票 03）：全局唯一 toast store（src/lib/toast.ts）
 * 的唯一渲染端，挂在应用根（App.vue），桌面端与网页端同一前端同享。
 * 形态：屏幕底部居中浮层，不挡操作区（容器 pointer-events 关闭、单条开启，
 * 浮层悬在设置弹窗等 overlay 之上）；成功绿档约 2.5 秒自动消失，失败红档
 * 约 5 秒、主文案带原因、可点 × 提前关。样式只用 App 根的深浅主题变量。
 */
import { dismissToast, toastItems } from "../lib/toast";
</script>

<template>
  <div class="toast-host" aria-live="polite">
    <div
      v-for="t in toastItems"
      :key="t.id"
      class="toast-item"
      :class="t.kind === 'success' ? 'toast-success' : 'toast-error'"
      role="status"
    >
      <div class="toast-body">
        <span class="toast-message">{{ t.message }}</span>
        <span v-if="t.reason" class="toast-reason">{{ t.reason }}</span>
      </div>
      <button class="toast-close" type="button" aria-label="关闭提示" @click="dismissToast(t.id)">
        ×
      </button>
    </div>
  </div>
</template>

<style scoped>
.toast-host {
  position: fixed;
  left: 50%;
  bottom: 18px;
  transform: translateX(-50%);
  /* 设置弹窗 overlay z-index 50：轻提示要浮在弹窗之上可见 */
  z-index: 60;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  max-width: min(520px, 92vw);
  /* 不挡操作区：容器不接事件，只有单条提示可点（× 按钮） */
  pointer-events: none;
}

.toast-item {
  pointer-events: auto;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 9px 12px;
  border-radius: 10px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  box-shadow: var(--shadow);
  font-size: 13px;
}

.toast-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.toast-message {
  font-weight: 600;
}

.toast-reason {
  font-size: 12px;
  word-break: break-all;
}

.toast-success {
  background: var(--ok-soft);
  border-color: var(--ok);
  color: var(--ok);
}

.toast-error {
  background: var(--bad-soft);
  border-color: var(--bad);
  color: var(--bad);
}

.toast-close {
  flex: none;
  border: none;
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 15px;
  line-height: 1;
  padding: 1px 2px;
  cursor: pointer;
  opacity: 0.7;
}

.toast-close:hover {
  opacity: 1;
}
</style>
