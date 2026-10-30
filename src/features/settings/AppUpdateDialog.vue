<script setup lang="ts">
import { computed } from "vue";
import { Dialog as KDialog, DialogButton as KDialogButton } from "konsta/vue";
import { appUpdate, currentVersion, ignoreAppUpdate, installAppUpdate, updateAction, updateBlocked, updateBusy,
  updateDialogOpen, updateError, updatePhase, updateProgress } from "./appUpdates";

const progressText = computed(() => {
  if (updatePhase.value === "installing") return "正在校验并安装更新…";
  if (updatePhase.value === "restart") return "更新已安装，重启应用后生效。";
  if (updatePhase.value === "permission") return "请在系统设置中允许安装此来源的应用，然后返回并继续安装。";
  if (updatePhase.value === "installer") return "请在系统安装器中完成更新。";
  if (updatePhase.value !== "downloading") return "";
  const progress = updateProgress.value;
  return progress?.total ? `正在下载 ${Math.min(100, Math.floor(progress.downloaded / progress.total * 100))}%`
    : `正在下载${progress ? ` ${(progress.downloaded / 1024 / 1024).toFixed(1)} MB` : "…"}`;
});
</script>

<template>
  <section v-if="updateDialogOpen && appUpdate" class="modal-backdrop z-[165]" @click.self="updateDialogOpen = false">
    <KDialog opened :backdrop="false" class="w-[min(480px,calc(100vw-32px))]"
      title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]">
      <template #title><h2>{{ updatePhase === 'restart' ? '更新已就绪' : '发现新版本' }}</h2></template>
      <p>{{ currentVersion ? `v${currentVersion} → ` : '' }}v{{ appUpdate.version }}</p>
      <div class="update-notes">{{ appUpdate.notes || '此版本包含功能改进与问题修复。' }}</div>
      <p v-if="appUpdate.kind === 'desktop' && !updateBusy && updatePhase !== 'restart'">安装完成后需要重启应用。</p>
      <p v-if="appUpdate.kind === 'android' && !updateBusy">下载并校验后，将打开系统安装器。</p>
      <p v-if="appUpdate.kind === 'ios'">此版本提供未签名 IPA，需要下载后自行签名安装。</p>
      <p v-if="progressText" role="status">{{ progressText }}</p>
      <p v-if="updateError" class="text-[var(--app-danger)]" role="alert">{{ updateError }}</p>
      <template #buttons>
        <div class="flex w-full flex-col gap-2 sm:flex-row sm:justify-end">
          <KDialogButton v-if="updatePhase === 'idle'" class="whitespace-nowrap" @click="ignoreAppUpdate">忽略此版本</KDialogButton>
          <KDialogButton class="whitespace-nowrap" data-back-action @click="updateDialogOpen = false">{{ updateBusy ? '后台进行' : '稍后' }}</KDialogButton>
          <KDialogButton strong class="whitespace-nowrap" :disabled="updateBlocked" @click="installAppUpdate">{{ updateBusy ? '更新中…' : updateAction }}</KDialogButton>
        </div>
      </template>
    </KDialog>
  </section>
</template>

<style scoped>
.update-notes { max-height: min(36dvh, 280px); overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; margin: 12px 0; }
</style>
