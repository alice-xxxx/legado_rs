<script setup lang="ts">
import { Button as KButton, Glass as KGlass } from "konsta/vue";
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
  <KGlass v-if="pendingImportLinks.length && !sourceImportOpen" class="external-import-banner material:bg-md-light-surface-2 material:dark:bg-md-dark-surface-2" role="status">
    <span class="external-import-banner__label">
      收到{{ pendingImportLinks[0] ? deepLinkLabels[pendingImportLinks[0].kind] : '' }}链接<span v-if="pendingImportLinks.length > 1">（待处理 {{ pendingImportLinks.length }} 个）</span>
    </span>
    <KButton rounded inline small class="gap-2" :disabled="importLinkBlocked" @click="openPendingImportLink">
      {{ importLinkOpening ? '正在打开…' : '打开确认页' }}
    </KButton>
    <KButton rounded inline tonal small class="gap-2" :disabled="importLinkOpening" @click="ignorePendingImportLink">忽略</KButton>
  </KGlass>
</template>

<style scoped>
@layer components {
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
  border-radius: 12px;
}

.external-import-banner__label { min-width: 160px; flex: 1; overflow-wrap: anywhere; }
}
</style>
