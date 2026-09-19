<script setup lang="ts">
/**
 * 网页端首启向导（webui-checkin 票 03）：新版首次启动弹一次（settings 键
 * webui_wizard_done，App 启动时查到未做才挂载本组件）。三步：
 * ① 选网段（NetBird 置顶，物理段强制明文确认）→ ② 生成凭证（查看/重生成）→
 * ③「完成」保存并触发防火墙（UAC 一次），随后展示完整地址复制。可跳过
 * （跳过同样写完成键，之后随时可在设置页开启）；子组件与设置「网页端」tab 共用。
 */
import { computed, onMounted, ref } from "vue";
import {
  getWebUiConfig,
  listNetworkSegments,
  markWebUiWizardDone,
  regenerateWebUiToken,
  saveWebUiConfig,
} from "../lib/ipc";
import type { NetworkSegment, WebUiConfigInfo, WebUiSaveOutcome } from "../types";
import WebUiSegmentPicker from "./WebUiSegmentPicker.vue";
import WebUiTokenArea from "./WebUiTokenArea.vue";
import WebUiAccessUrl from "./WebUiAccessUrl.vue";

const emit = defineEmits<{ close: [] }>();

const step = ref(1);
const config = ref<WebUiConfigInfo | null>(null);
const segments = ref<NetworkSegment[]>([]);
const selected = ref<string[]>([]);
const busy = ref(false);
const errorText = ref("");
const outcome = ref<WebUiSaveOutcome | null>(null);
/** 完成保存后进入收尾视图（展示地址与防火墙结果）。 */
const finished = ref(false);
const urlKey = ref(0);

onMounted(async () => {
  try {
    const [cfg, segs] = await Promise.all([getWebUiConfig(), listNetworkSegments()]);
    segments.value = Array.isArray(segs) ? segs : [];
    if (cfg && typeof cfg === "object") {
      config.value = cfg;
      selected.value = [...cfg.segments];
    }
  } catch (e) {
    errorText.value = String(e);
  }
});

const canNext = computed(() => selected.value.length > 0);

function next() {
  if (canNext.value && step.value < 3) step.value++;
}

function back() {
  if (step.value > 1 && !finished.value) step.value--;
}

async function regenerate() {
  errorText.value = "";
  try {
    config.value = await regenerateWebUiToken();
  } catch (e) {
    errorText.value = String(e);
  }
}

/** 第 ③ 步「完成」：保存（enabled=true + 所选网段 + 当前端口）→ Rust 联动防火墙。 */
async function finish() {
  busy.value = true;
  errorText.value = "";
  try {
    outcome.value = await saveWebUiConfig({
      input: {
        enabled: true,
        segments: selected.value,
        port: config.value?.port ?? 17321,
      },
    });
    config.value = outcome.value.config;
    urlKey.value++;
    finished.value = true;
  } catch (e) {
    errorText.value = String(e);
  } finally {
    busy.value = false;
  }
}

/** 完成/跳过共用收尾：写完成键后关闭（写键失败不挡关闭——下次启动会再弹，可接受）。 */
async function closeAndMark() {
  try {
    await markWebUiWizardDone();
  } catch {
    // 静默：完成键写失败只影响「不再弹」，不影响本次功能
  }
  emit("close");
}
</script>

<template>
  <div class="overlay" @click.self="closeAndMark">
    <div class="dialog webui-wizard">
      <h3>网页端 · 首次设置向导</h3>

      <!-- 步骤 ①：选网段 -->
      <div v-if="!finished && step === 1" class="wiz-step wiz-step-1">
        <p class="wiz-lead">
          启用后，手机可以在所选网段内打开网页版（打卡 / 历史 / 巢况）。
          建议只勾选 NetBird 虚拟网；勾选物理网段会明文传输凭证与数据。
        </p>
        <WebUiSegmentPicker v-model:selected="selected" :segments="segments" />
        <p v-if="!canNext" class="wiz-hint">先勾选至少一个网段才能继续。</p>
      </div>

      <!-- 步骤 ②：生成凭证 -->
      <div v-else-if="step === 2" class="wiz-step wiz-step-2">
        <p class="wiz-lead">访问凭证已由程序生成——手机打开带凭证的完整地址即用，永不输密码。</p>
        <WebUiTokenArea
          :token="config?.token ?? ''"
          :generated-at="config?.token_generated_at ?? null"
          @regenerate="regenerate"
        />
        <p v-if="errorText" class="form-error">{{ errorText }}</p>
      </div>

      <!-- 步骤 ③：完成（触发防火墙）→ 收尾视图（地址复制 + 防火墙结果） -->
      <div v-else class="wiz-step wiz-step-3">
        <template v-if="!finished">
          <p class="wiz-lead">
            点「完成」保存设置：Windows 会弹一次防火墙授权（UAC），请选「是」。
          </p>
          <p v-if="errorText" class="form-error">{{ errorText }}</p>
        </template>
        <template v-else>
          <p v-if="outcome?.firewall_ok" class="wiz-fw-ok">
            防火墙已放行所选网段（仅限 {{ outcome.config.segments.join("、") }}，端口
            {{ outcome.config.port }}）。
          </p>
          <div v-else-if="outcome" class="firewall-fail">
            <p class="form-error">{{ outcome.firewall_error }}</p>
            <pre class="firewall-manual">{{ outcome.firewall_manual_cmd }}</pre>
            <p class="wiz-hint">可先跳过，稍后在设置页重新保存即可再次同步。</p>
          </div>
          <WebUiAccessUrl :refresh-key="urlKey" />
        </template>
      </div>

      <div class="dlg-btns">
        <button v-if="!finished" class="btn wiz-skip-btn" type="button" :disabled="busy" @click="closeAndMark">
          跳过（暂不启用）
        </button>
        <span class="spacer"></span>
        <button v-if="step > 1 && !finished" class="btn" type="button" :disabled="busy" @click="back">
          上一步
        </button>
        <button
          v-if="step === 1 && !finished"
          class="btn primary wiz-next-btn"
          type="button"
          :disabled="!canNext"
          @click="next"
        >
          下一步
        </button>
        <button
          v-else-if="step === 2 && !finished"
          class="btn primary wiz-next-btn"
          type="button"
          @click="next"
        >
          下一步
        </button>
        <button
          v-else-if="!finished"
          class="btn primary wiz-finish-btn"
          type="button"
          :disabled="busy"
          @click="finish"
        >
          完成（保存并放行防火墙）
        </button>
        <button v-else class="btn primary wiz-close-btn" type="button" @click="closeAndMark">
          完成
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.webui-wizard {
  width: 560px;
  max-width: 94vw;
  max-height: 86vh;
  overflow-y: auto;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.webui-wizard h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

.wiz-lead {
  font-size: 13px;
  color: var(--muted);
  margin-bottom: 10px;
}

.wiz-hint {
  font-size: 12px;
  color: var(--muted);
  margin-top: 6px;
}

.wiz-fw-ok {
  font-size: 13px;
  color: var(--ok, #188a4b);
  margin-bottom: 10px;
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

.dlg-btns {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 16px;
}

.spacer {
  flex: 1;
}

.btn {
  padding: 7px 18px;
  border-radius: 9px;
  border: 1px solid var(--border-strong);
  background: var(--card);
  cursor: pointer;
  font: inherit;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
  font-weight: 600;
}

.btn.primary:hover {
  background: var(--accent-deep);
}

.btn:disabled {
  opacity: 0.6;
  cursor: default;
}

.form-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
}

.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 60;
}
</style>
