<script setup lang="ts">
/**
 * 访问凭证区（webui-checkin 票 03）：打码显示 + 查看/隐藏切换 + 重生成（两段
 * 确认，明示旧地址立即作废）。凭证只由程序随机生成，界面不提供自定义输入
 * （规格 B：杜绝弱凭证）；重生成事件交父组件走 regenerate_token 命令。
 * 设置「网页端」tab 与首启向导共用。
 */
import { computed, ref } from "vue";
import { REGEN_WARNING, maskToken } from "../lib/webuiUi";

const props = defineProps<{ token: string; generatedAt: string | null }>();
const emit = defineEmits<{ regenerate: [] }>();

const revealed = ref(false);
/** 两段确认：第一次点「重新生成」只进入确认态（与删除记录同待遇）。 */
const confirming = ref(false);

const display = computed(() => (revealed.value ? props.token : maskToken(props.token)));

function toggleReveal() {
  revealed.value = !revealed.value;
}

function askRegenerate() {
  if (!confirming.value) {
    confirming.value = true;
    return;
  }
  confirming.value = false;
  emit("regenerate");
}

function cancelRegenerate() {
  confirming.value = false;
}
</script>

<template>
  <div class="token-area">
    <div class="token-row">
      <span class="token-caption">访问凭证：</span>
      <code v-if="!revealed" class="token-masked">{{ display }}</code>
      <code v-else class="token-plain">{{ display }}</code>
      <button class="btn token-toggle" type="button" @click="toggleReveal">
        {{ revealed ? "隐藏" : "查看明文" }}
      </button>
      <button class="btn token-regen" type="button" @click="askRegenerate">重新生成</button>
    </div>
    <p class="token-meta">
      凭证由程序随机生成（不可自定义）；{{ generatedAt ? `生成于 ${generatedAt}` : "尚未生成" }}。
    </p>
    <div v-if="confirming" class="regen-confirm">
      <p class="regen-warning">{{ REGEN_WARNING }}</p>
      <div class="dlg-btns">
        <button class="btn regen-cancel" type="button" @click="cancelRegenerate">取消</button>
        <button class="btn regen-ok" type="button" @click="askRegenerate">确认重生成（旧地址作废）</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.token-area {
  margin-bottom: 10px;
}

.token-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.token-caption {
  font-size: 14px;
}

.token-masked,
.token-plain {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 13px;
  background: var(--tile);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 4px 10px;
  word-break: break-all;
}

.token-plain {
  color: var(--bad);
}

.token-meta {
  margin-top: 6px;
  font-size: 12px;
  color: var(--muted);
}

.regen-confirm {
  border: 1px solid var(--border-strong);
  border-left: 3px solid var(--bad);
  border-radius: 8px;
  padding: 8px 10px;
  background: var(--tile);
  margin-top: 8px;
}

.regen-warning {
  font-size: 13px;
  color: var(--bad);
  margin-bottom: 8px;
}

.regen-confirm .dlg-btns {
  margin-top: 0;
  justify-content: flex-end;
}
</style>
