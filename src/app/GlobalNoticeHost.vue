<script setup lang="ts">
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

  <transition name="toast">
    <div v-if="toast" class="toast-message" :class="toastKind" role="status">
      <span>{{ toastKind === "success" ? "✓" : "!" }}</span>{{ toast }}
      <button type="button" aria-label="关闭提示" @click="dismissToast">×</button>
    </div>
  </transition>
</template>

<style scoped>
.shelf-batch-recovery{display:flex;align-items:flex-start;gap:10px;margin:12px auto;padding:12px 16px;border:1px solid #e8c5c8;border-radius:10px;color:#8d4b50;background:#fff7f7}
.shelf-batch-recovery>span{flex:1}
.toast-message{position:fixed;z-index:90;top:max(12px,env(safe-area-inset-top));left:50%;display:flex;max-width:min(480px,calc(100vw - 24px));align-items:flex-start;gap:8px;padding:10px 14px;transform:translateX(-50%);border:1px solid var(--app-line);border-radius:12px;color:var(--app-text);background:var(--app-surface);box-shadow:var(--app-shadow);overflow-wrap:anywhere}
.toast-message.error{border-color:#e8c5c8}.toast-message.success{border-color:#c7dfcf}
.toast-message button{border:0;background:transparent;color:inherit;cursor:pointer}
.toast-enter-active,.toast-leave-active{transition:opacity 140ms ease,transform 140ms ease}
.toast-enter-from,.toast-leave-to{opacity:0;transform:translate(-50%,-6px)}
</style>
