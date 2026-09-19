<script setup lang="ts">
/**
 * 设置弹窗「网页端」tab（webui-checkin 票 03）：总开关（默认关）/ 受信网段多选 /
 * 端口 / 凭证区 / 完整地址。保存走 save_webui_config（Rust 落数据目录
 * webui-config.json + 同步防火墙规则 + 票 04 起按新配置起停内嵌 HTTP 服务）。
 * 半成功语义：防火墙失败 / 端口占用等只提示、不吞掉已保存的配置——展示错误
 * （+ 防火墙的现成 netsh 手动命令与复制）。
 * 首启向导（WebUiWizard）复用本面板的三个子组件，不复制粘贴。
 */
import { onMounted, ref } from "vue";
import {
  getWebUiConfig,
  listNetworkSegments,
  regenerateWebUiToken,
  saveWebUiConfig,
} from "../lib/ipc";
import type { NetworkSegment, WebUiConfigInfo, WebUiSaveOutcome } from "../types";
import { PORT_ERROR, copyText, validatePortText } from "../lib/webuiUi";
import WebUiSegmentPicker from "./WebUiSegmentPicker.vue";
import WebUiTokenArea from "./WebUiTokenArea.vue";
import WebUiAccessUrl from "./WebUiAccessUrl.vue";

const emit = defineEmits<{ changed: [] }>();

const config = ref<WebUiConfigInfo | null>(null);
const segments = ref<NetworkSegment[]>([]);
const form = ref<{ enabled: boolean; selected: string[]; portText: string }>({
  enabled: false,
  selected: [],
  portText: "17321",
});
const errorText = ref("");
const savedText = ref("");
const busy = ref(false);
const lastOutcome = ref<WebUiSaveOutcome | null>(null);
/** 保存/重生成后让地址区重拉（子组件 watch 该键）。 */
const urlKey = ref(0);
const manualCopied = ref(false);

async function load() {
  const [cfg, segs] = await Promise.all([getWebUiConfig(), listNetworkSegments()]);
  segments.value = Array.isArray(segs) ? segs : [];
  if (cfg && typeof cfg === "object") {
    config.value = cfg;
    form.value = {
      enabled: cfg.enabled,
      selected: [...cfg.segments],
      portText: String(cfg.port),
    };
  }
}

onMounted(async () => {
  try {
    await load();
  } catch (e) {
    errorText.value = String(e);
  }
});

async function regenerate() {
  errorText.value = "";
  savedText.value = "";
  try {
    config.value = await regenerateWebUiToken();
    urlKey.value++;
    savedText.value = "凭证已重生成，旧地址即刻作废";
    emit("changed");
  } catch (e) {
    errorText.value = String(e);
  }
}

async function save() {
  const port = validatePortText(form.value.portText);
  if (port === null) {
    errorText.value = PORT_ERROR;
    savedText.value = "";
    return;
  }
  busy.value = true;
  errorText.value = "";
  savedText.value = "";
  manualCopied.value = false;
  try {
    const outcome = await saveWebUiConfig({
      input: { enabled: form.value.enabled, segments: form.value.selected, port },
    });
    lastOutcome.value = outcome;
    config.value = outcome.config;
    form.value = {
      enabled: outcome.config.enabled,
      selected: [...outcome.config.segments],
      portText: String(outcome.config.port),
    };
    urlKey.value++;
    savedText.value = "已保存";
    emit("changed");
  } catch (e) {
    errorText.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function copyManual() {
  const cmd = lastOutcome.value?.firewall_manual_cmd;
  if (!cmd) return;
  manualCopied.value = await copyText(cmd);
}
</script>

<template>
  <div class="webui-panel">
    <div class="webui-row">
      <label>
        <input v-model="form.enabled" class="webui-enabled-input" type="checkbox" />
        启用网页端（局域网内手机访问；端口与网段保存后同步防火墙规则）
      </label>
    </div>

    <div class="webui-row">
      <p class="webui-seg-title">受信网段（NetBird 段自动识别置顶；勾选物理网段会要求确认明文风险）：</p>
      <WebUiSegmentPicker v-model:selected="form.selected" :segments="segments" />
    </div>

    <div class="webui-row">
      <label>
        端口
        <input
          v-model="form.portText"
          class="webui-port-input"
          type="number"
          min="1024"
          max="65535"
          title="1024–65535；端口被占用时服务不启动，保存后会在这里提示"
        />
      </label>
    </div>

    <WebUiTokenArea
      :token="config?.token ?? ''"
      :generated-at="config?.token_generated_at ?? null"
      @regenerate="regenerate"
    />

    <WebUiAccessUrl :refresh-key="urlKey" />

    <p v-if="errorText" class="form-error webui-error">{{ errorText }}</p>
    <p v-if="savedText" class="saved-hint webui-saved">{{ savedText }}</p>

    <!-- 防火墙联动失败（半成功语义）：配置已保存，这里给现成手动命令 -->
    <div v-if="lastOutcome && !lastOutcome.firewall_ok" class="firewall-fail">
      <p class="form-error firewall-fail-msg">{{ lastOutcome.firewall_error }}</p>
      <pre class="firewall-manual">{{ lastOutcome.firewall_manual_cmd }}</pre>
      <button class="btn copy-manual-btn" type="button" @click="copyManual">复制手动命令</button>
      <span v-if="manualCopied" class="copy-ok">已复制</span>
    </div>

    <!-- 服务起停失败（半成功语义，票 04）：端口被占用等，配置已照常保存 -->
    <p v-if="lastOutcome && !lastOutcome.server_ok" class="form-error server-fail-msg">
      {{ lastOutcome.server_error }}
    </p>

    <div class="dlg-btns">
      <span class="spacer"></span>
      <button class="btn primary save-webui-btn" type="button" :disabled="busy" @click="save">
        保存
      </button>
    </div>
  </div>
</template>

<style scoped>
.webui-row {
  margin-bottom: 10px;
  font-size: 14px;
}

.webui-row label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}

.webui-seg-title {
  font-size: 13px;
  color: var(--muted);
  margin-bottom: 6px;
}

.webui-port-input {
  width: 84px;
  padding: 6px 8px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.firewall-fail {
  border: 1px solid var(--border-strong);
  border-left: 3px solid var(--bad);
  border-radius: 8px;
  padding: 8px 10px;
  background: var(--tile);
  margin-bottom: 10px;
}

.firewall-manual {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 6px 10px;
  margin-bottom: 8px;
}

.copy-ok {
  margin-left: 8px;
  font-size: 12px;
  color: var(--ok, #188a4b);
}
</style>
