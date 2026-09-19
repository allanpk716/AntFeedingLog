<script setup lang="ts">
/**
 * 受信网段多选（webui-checkin 票 03）：设置「网页端」tab 与首启向导共用。
 * - 列表来自 list_network_segments（Rust 已滤虚拟化噪音、NetBird 段置顶标名）；
 * - NetBird 段带「已确认加密」标名；物理网段标「明文」；
 * - **勾任何 encrypted_mesh=false 段 → 内联硬警示确认框**（规格 B：明示该网段内
 *   凭证与数据将明文传输，不确认不给勾），取消即不勾；NetBird 段与取消勾选即时生效；
 * - 同一时刻至多一个待确认段（再勾另一物理段会替换待确认对象）。
 */
import { ref } from "vue";
import type { NetworkSegment } from "../types";
import { PLAINTEXT_WARNING } from "../lib/webuiUi";

const props = defineProps<{ segments: NetworkSegment[]; selected: string[] }>();
const emit = defineEmits<{ "update:selected": [value: string[]] }>();

/** 待确认明文风险的物理网段 CIDR（null = 无待确认）。 */
const pendingCidr = ref<string | null>(null);

function isSelected(cidr: string): boolean {
  return props.selected.includes(cidr);
}

function toggle(seg: NetworkSegment) {
  if (isSelected(seg.cidr)) {
    // 取消勾选即时生效
    if (pendingCidr.value === seg.cidr) pendingCidr.value = null;
    emit("update:selected", props.selected.filter((c) => c !== seg.cidr));
    return;
  }
  if (seg.encrypted_mesh) {
    emit("update:selected", [...props.selected, seg.cidr]);
    return;
  }
  pendingCidr.value = seg.cidr; // 物理网段：先确认明文风险，不确认不给勾
}

function confirmPending() {
  if (pendingCidr.value && !isSelected(pendingCidr.value)) {
    emit("update:selected", [...props.selected, pendingCidr.value]);
  }
  pendingCidr.value = null;
}

function cancelPending() {
  pendingCidr.value = null;
}
</script>

<template>
  <div class="seg-picker">
    <div
      v-for="seg in segments"
      :key="seg.cidr"
      class="seg-row"
      :class="{ 'seg-encrypted': seg.encrypted_mesh }"
    >
      <label class="seg-label-row">
        <input class="seg-check" type="checkbox" :checked="isSelected(seg.cidr)" @change="toggle(seg)" />
        <span class="seg-cidr">{{ seg.cidr }}</span>
        <span v-if="seg.label" class="seg-tag encrypted">{{ seg.label }} · 已确认加密</span>
        <span v-else class="seg-tag plain">物理网段 · 明文</span>
      </label>
      <div v-if="pendingCidr === seg.cidr" class="seg-confirm">
        <p class="seg-confirm-text">{{ PLAINTEXT_WARNING }}</p>
        <div class="seg-confirm-btns">
          <button class="btn confirm-no" type="button" @click="cancelPending">取消</button>
          <button class="btn confirm-yes" type="button" @click="confirmPending">确认信任该网段</button>
        </div>
      </div>
    </div>
    <p v-if="segments.length === 0" class="seg-empty">没有发现可用网段（虚拟化网卡已过滤）</p>
  </div>
</template>

<style scoped>
.seg-picker {
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 8px 12px;
  margin-bottom: 10px;
}

.seg-row {
  padding: 4px 0;
}

.seg-label-row {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 14px;
}

.seg-cidr {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 13px;
}

.seg-tag {
  font-size: 11px;
  border-radius: 999px;
  padding: 1px 8px;
  white-space: nowrap;
}

.seg-tag.encrypted {
  color: var(--ok, #188a4b);
  background: var(--ok-soft, #e7f5ec);
}

.seg-tag.plain {
  color: var(--muted);
  background: var(--tile);
}

.seg-confirm {
  margin: 6px 0 4px 24px;
  border: 1px solid var(--border-strong);
  border-left: 3px solid var(--bad);
  border-radius: 8px;
  padding: 8px 10px;
  background: var(--tile);
}

.seg-confirm-text {
  font-size: 13px;
  color: var(--bad);
  margin-bottom: 8px;
}

.seg-confirm-btns {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}

.seg-empty {
  font-size: 13px;
  color: var(--muted);
}
</style>
