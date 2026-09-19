<script setup lang="ts">
/**
 * 快捷打卡面板（反馈第二轮 F1，Q1=C）：非喂食操作点击后先弹此面板——
 * 日期默认今天、可补录过去、可填备注，点「记录」才落库；取消不产生任何记录。
 * 喂食不弹这里（FeedDialog 自带食物多选）。后端 log_care 已拒未来时间。
 */
import { ref } from "vue";
import { logCare } from "../lib/ipc";
import type { Colony, ColonyAction } from "../types";
import { nowLocalDateTime } from "../lib/care";

const props = defineProps<{ colony: Colony; action: ColonyAction }>();
const emit = defineEmits<{ close: []; saved: [] }>();

const time = ref(nowLocalDateTime());
const note = ref("");
const formError = ref("");
const busy = ref(false);

async function submit() {
  busy.value = true;
  formError.value = "";
  try {
    await logCare({
      input: {
        colony_id: props.colony.id,
        action_id: props.action.action_id,
        happened_at: time.value,
        note: note.value.trim() === "" ? null : note.value.trim(),
        food_ids: [],
      },
    });
    emit("saved");
  } catch (e) {
    formError.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="overlay" @click.self="$emit('close')">
    <div class="dialog quick-dialog vp-dialog">
      <h3>记录{{ action.name }} · {{ colony.name }}</h3>

      <div class="field-label">时间（默认现在，可补录）</div>
      <input v-model="time" class="time-input" type="datetime-local" />

      <div class="field-label">备注（可选）</div>
      <textarea v-model="note" class="note-input" placeholder="如：顺手检查了垃圾区"></textarea>

      <p v-if="formError" class="form-error">{{ formError }}</p>

      <div class="dlg-btns">
        <button class="btn cancel-btn" type="button" @click="$emit('close')">取消</button>
        <button class="btn primary record-btn" type="button" :disabled="busy" @click="submit">
          记录
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 视觉基线 mocks/mock-a-light.html 快捷打卡面板（同 FeedDialog 弹窗骨架） */
.overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.dialog {
  width: 390px;
  max-width: 92vw;
  background: var(--card);
  border-radius: 14px;
  padding: 18px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.dialog h3 {
  font-size: 15px;
  margin-bottom: 12px;
}

.field-label {
  font-size: 12px;
  color: var(--muted);
  margin: 12px 0 6px;
}

.dialog input[type="datetime-local"],
.dialog textarea {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  font: inherit;
  background: var(--card);
  color: var(--text);
}

.dialog textarea {
  height: 56px;
  resize: none;
  margin-top: 2px;
}

.form-error {
  margin-top: 10px;
  font-size: 13px;
  color: var(--bad);
}

.dlg-btns {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
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

/* ── 手机竖屏（webui-checkin 票 08）：≤480px 输入放大 + 大号按钮；
   vp-dialog 是媒体查询落点（断点类名供组件测试断言——jsdom 不套用媒体查询）── */
@media (max-width: 480px) {
  .vp-dialog.dialog {
    padding: 14px;
  }

  .vp-dialog input[type="datetime-local"],
  .vp-dialog textarea {
    padding: 11px 12px;
    font-size: 16px; /* ≥16px 防 iOS 聚焦自动放大 */
  }

  .vp-dialog .btn {
    padding: 11px 22px;
    font-size: 15px;
  }

  .vp-dialog .dlg-btns {
    flex-direction: row-reverse; /* 主按钮在拇指侧 */
  }
}
</style>
