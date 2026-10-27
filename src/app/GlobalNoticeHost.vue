<script setup lang="ts">
import { Toast as KToast, Button as KButton } from "konsta/vue";
import { dismissToast, toast, toastKind } from "./notifications";
import { shelfBatchRecoveryMessage, shelfBatchRecoveryRequired, shelfBatchRecoveryTitle } from "./recoveryState";
import { getReaderSession } from "../features/reader/readerSession";

const readerVisible = getReaderSession().readerVisible;
</script>

<template>
  <div v-if="shelfBatchRecoveryRequired && !readerVisible" class="shelf-batch-recovery" role="status">
    <strong>{{ shelfBatchRecoveryTitle }}</strong>
    <span>{{ shelfBatchRecoveryMessage || "为避免重复修改，本会话已停止书架批量操作。重启后再查看书架状态。" }}</span>
  </div>

  <KToast :opened="!!toast" position="center" class="app-toast z-[90]" role="status" aria-live="polite">
    <span class="min-w-0 whitespace-pre-wrap break-words">{{ toastKind === "success" ? "✓" : "!" }} {{ toast }}</span>
    <template #button><KButton rounded inline clear type="button" aria-label="关闭提示" @click="dismissToast">关闭</KButton></template>
  </KToast>
</template>

<style scoped>
@layer components {
.shelf-batch-recovery{display:flex;align-items:flex-start;gap:10px;margin:12px auto;padding:12px 16px;border:1px solid var(--app-danger-line);border-radius:10px;color:var(--app-danger);background:var(--app-panel-soft)}
.shelf-batch-recovery>span{flex:1}
.app-toast { bottom: calc(var(--ui-nav-height) + var(--safe-bottom) + 16px); }
}
</style>
