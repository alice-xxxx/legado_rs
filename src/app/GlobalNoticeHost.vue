<script setup lang="ts">
import { Toast as KToast, Button as KButton, Dialog as KDialog, DialogButton as KDialogButton } from "konsta/vue";
import { confirmation, answerConfirmation } from "./confirmation";
import { dismissToast, toast, toastKind } from "./notifications";
import { shelfBatchRecoveryMessage, shelfBatchRecoveryRequired, shelfBatchRecoveryTitle } from "./recoveryState";
import { getReaderSession } from "../features/reader/readerSession";

const readerVisible = getReaderSession().readerVisible;
</script>

<template>
  <section v-if="confirmation" class="modal-backdrop z-[170]" @click.self="answerConfirmation(false)">
    <KDialog opened :backdrop="false" class="w-[min(420px,calc(100vw-32px))]">
      <template #title><h2>{{ confirmation.title }}</h2></template>
      <p>{{ confirmation.message }}</p>
      <template #buttons>
        <KDialogButton data-back-action @click="answerConfirmation(false)">取消</KDialogButton>
        <KDialogButton strong class="k-color-brand-danger" @click="answerConfirmation(true)">{{ confirmation.confirmLabel }}</KDialogButton>
      </template>
    </KDialog>
  </section>
  <div v-if="shelfBatchRecoveryRequired && !readerVisible" class="shelf-batch-recovery" role="status">
    <strong>{{ shelfBatchRecoveryTitle }}</strong>
    <span>{{ shelfBatchRecoveryMessage || "为避免重复修改，本会话已停止书架批量操作。重启后再查看书架状态。" }}</span>
  </div>

  <KToast :opened="!!toast" position="center" class="app-toast z-[180]" :role="toastKind === 'error' ? 'alert' : 'status'" :aria-live="toastKind === 'error' ? 'assertive' : 'polite'">
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
