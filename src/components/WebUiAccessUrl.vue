<script setup lang="ts">
/**
 * 完整访问地址区（webui-checkin 票 03）：`http://<IP>:<端口>/#token=<凭证>` 展示
 * + 一键复制（复制含 fragment 凭证的完整地址）+ 拉取失败原因（网段不在线等）。
 * refreshKey 变化（保存/重生成后）重新拉取；设置「网页端」tab 与首启向导共用。
 */
import { onMounted, ref, watch } from "vue";
import { getWebUiAccessUrl } from "../lib/ipc";
import { RISK_HINT, copyText } from "../lib/webuiUi";

const props = defineProps<{ refreshKey?: number }>();

const url = ref("");
const errorText = ref("");
const copied = ref(false);
const copyFailed = ref(false);
/** 竞态保护：只采纳最后一次拉取的结果（refreshKey 连变时旧响应不回写）。 */
let loadSeq = 0;

async function load() {
  const seq = ++loadSeq;
  errorText.value = "";
  url.value = "";
  try {
    const u = await getWebUiAccessUrl();
    if (seq !== loadSeq) return;
    url.value = u;
  } catch (e) {
    if (seq !== loadSeq) return;
    errorText.value = String(e);
  }
}

async function copyUrl() {
  copied.value = false;
  copyFailed.value = false;
  const ok = await copyText(url.value);
  if (ok) {
    copied.value = true;
  } else {
    copyFailed.value = true;
  }
}

onMounted(load);
watch(
  () => props.refreshKey,
  () => void load(),
);
</script>

<template>
  <div class="access-url-box">
    <p class="access-url-caption">手机访问地址（存成书签，打开即用、永不输密码）：</p>
    <p v-if="url" class="access-url">{{ url }}</p>
    <p v-else-if="errorText" class="form-error access-url-error">{{ errorText }}</p>
    <div v-if="url" class="url-actions">
      <button class="btn copy-url-btn" type="button" @click="copyUrl">复制完整地址</button>
      <span v-if="copied" class="copy-ok">已复制（含进门凭证，注意保管）</span>
      <span v-else-if="copyFailed" class="copy-fail">复制失败，请手动选中复制</span>
    </div>
    <p class="hint risk-hint">{{ RISK_HINT }}</p>
  </div>
</template>

<style scoped>
.access-url-box {
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 10px 12px;
  margin-bottom: 10px;
}

.access-url-caption {
  font-size: 13px;
  margin-bottom: 4px;
}

.access-url {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 13px;
  word-break: break-all;
  background: var(--tile);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 6px 10px;
}

.url-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
}

.copy-ok {
  font-size: 12px;
  color: var(--ok, #188a4b);
}

.copy-fail {
  font-size: 12px;
  color: var(--bad);
}
</style>
