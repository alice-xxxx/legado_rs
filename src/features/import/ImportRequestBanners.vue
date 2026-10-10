<script setup lang="ts">
import type { PendingExternalFile } from "../../api/types";

defineProps<{
  pendingExternalFiles: PendingExternalFile[];
  externalFileRequestBusy: boolean;
  externalFileRequestBlocked: boolean;
  pendingImportLinks: Array<{ kind: "source" | "rules"; url: string }>;
  importLinkOpening: boolean;
  importLinkBlocked: boolean;
  recoveryRequired: boolean;
  sourceImportOpen: boolean;
  acceptExternalFileRequest: () => void | Promise<void>;
  ignoreExternalFileRequest: () => void | Promise<void>;
  openPendingImportLink: () => void | Promise<void>;
  ignorePendingImportLink: () => void;
}>();
</script>

<template>
  <div v-if="pendingExternalFiles.length" class="external-import-banner">
    <span class="external-import-banner__label">
      打开文件：{{ pendingExternalFiles[0]?.filename }}<span v-if="pendingExternalFiles.length > 1">（待处理 {{ pendingExternalFiles.length }} 个）</span>
    </span>
    <button class="button primary small" :disabled="externalFileRequestBlocked" @click="acceptExternalFileRequest">
      {{ externalFileRequestBusy ? '处理中…' : '导入书架' }}
    </button>
    <button class="button secondary small" :disabled="externalFileRequestBusy || recoveryRequired" @click="ignoreExternalFileRequest">忽略</button>
  </div>

  <div v-if="pendingImportLinks.length && !pendingExternalFiles.length && !sourceImportOpen" class="external-import-banner">
    <span class="external-import-banner__label">
      收到{{ pendingImportLinks[0]?.kind === 'source' ? '书源' : '显示替换规则' }}导入链接<span v-if="pendingImportLinks.length > 1">（待处理 {{ pendingImportLinks.length }} 个）</span>
    </span>
    <button class="button primary small" :disabled="importLinkBlocked" @click="openPendingImportLink">
      {{ importLinkOpening ? '正在打开…' : '打开确认页' }}
    </button>
    <button class="button secondary small" @click="ignorePendingImportLink">忽略</button>
  </div>
</template>

<style scoped>
.external-import-banner {
  position: fixed;
  z-index: 60;
  top: max(12px, env(safe-area-inset-top));
  left: 50%;
  display: flex;
  width: min(540px, calc(100vw - 24px));
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  padding: 14px;
  transform: translateX(-50%);
  border: 1px solid var(--app-line);
  border-radius: 12px;
  background: var(--app-surface);
  box-shadow: 0 6px 24px #0002;
}

.external-import-banner__label { min-width: 160px; flex: 1; overflow-wrap: anywhere; }
</style>
