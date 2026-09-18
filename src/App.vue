<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

/**
 * 健康检查拿到的 schema 版本。
 * null = IPC 未返回（前端单测环境或启动早期），首页照常渲染空状态。
 */
const schemaVersion = ref<number | null>(null);

onMounted(async () => {
  try {
    const info = await invoke<{ schema_version: number }>("health_check");
    schemaVersion.value = info.schema_version;
  } catch (err) {
    console.error("健康检查失败", err);
  }
});
</script>

<template>
  <main class="container">
    <!-- 真正的首页卡片墙在票 02 实现；本票只放空状态占位 -->
    <p class="empty">暂无窝</p>
    <p v-if="schemaVersion !== null" class="schema">数据 schema v{{ schemaVersion }}</p>
  </main>
</template>

<style scoped>
.container {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.empty {
  font-size: 18px;
  color: #999;
}

.schema {
  font-size: 12px;
  color: #bbb;
  margin-top: 8px;
}
</style>
