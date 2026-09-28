<script setup lang="ts">
/**
 * 全局唯一加载占位组件（界面切换卡顿票 01）：异步读取完成前的诚实占位，
 * 渲染一行低调的「加载中…」文字，读取完成即由调用方摘除。无 props、无
 * emits、无业务逻辑；样式与 .hint/.wall-empty 一族（小字、muted 色）。
 * 判定原则见 CLAUDE.md「加载态规范」：数据已在内存时直接渲染，不进加载态。
 */
</script>

<template>
  <p class="loading-hint">加载中…</p>
</template>

<style scoped>
.loading-hint {
  font-size: 13px;
  color: var(--muted, #8f887d);
}
</style>
