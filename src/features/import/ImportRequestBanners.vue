<script setup lang="ts">
import { deepLinkLabels, type PendingImportLink } from "./deepLinks";

defineProps<{
  pendingImportLinks: PendingImportLink[];
  importLinkOpening: boolean;
  importLinkBlocked: boolean;
  recoveryRequired: boolean;
  sourceImportOpen: boolean;
  openPendingImportLink: () => void | Promise<void>;
  ignorePendingImportLink: () => void;
}>();
</script>

<template>
  <div v-if="pendingImportLinks.length && !sourceImportOpen" class="external-import-banner">
    <span class="external-import-banner__label">
      收到{{ pendingImportLinks[0] ? deepLinkLabels[pendingImportLinks[0].kind] : '' }}链接<span v-if="pendingImportLinks.length > 1">（待处理 {{ pendingImportLinks.length }} 个）</span>
    </span>
    <button class="button primary small" :disabled="importLinkBlocked" @click="openPendingImportLink">
      {{ importLinkOpening ? '正在打开…' : '打开确认页' }}
    </button>
    <button class="button secondary small" :disabled="importLinkOpening" @click="ignorePendingImportLink">忽略</button>
  </div>
</template>

<style scoped>
.external-import-banner {
  position: fixed;
  z-index: 60;
  top: max(12px, var(--safe-top));
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
