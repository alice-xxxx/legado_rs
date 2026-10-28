<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref } from "vue";
import { Glass as KGlass, Progressbar as KProgressbar } from "konsta/vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppBootstrap } from "../../api/types";
import { notify } from "../../app/notifications";
import { getFrontendRestoreRevision, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { getReaderSession } from "../reader/readerSession";
import { useLocalBookImport } from "../books/useLocalBookImport";
import router, { routeNames } from "../../router";
import { useExternalImportActions } from "./useExternalImportActions";
import { installLocalBookImportSession } from "./localBookImportSession";
import ImportRequestBanners from "./ImportRequestBanners.vue";
import DeepLinkImportDialog from "./DeepLinkImportDialog.vue";

const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);
const bootstrapped = ref(false);
const currentRoute = router.currentRoute;
const sourceImportOpen = computed(() => currentRoute.value.name === routeNames.sources
  && (currentRoute.value.query.sourceImport === "url" || currentRoute.value.query.sourceImport === "new"));
const unlisteners: UnlistenFn[] = [];
const stages = { read: "读取文件", copy: "复制文件", parse: "解析文件", cache: "生成缓存", save: "登记书架", done: "完成", error: "失败" };
let disposed = false;


function refreshRestoredShelf(_snapshot: AppBootstrap): void {
  externalImports.cancelPendingImport();
  bootstrapped.value = true;
}

const localBookImportFeature = useLocalBookImport({ notify });
const removeLocalBookImportSession = installLocalBookImportSession({
  localBookImportBusy: localBookImportFeature.localBookImportBusy,
  importLocalBook: localBookImportFeature.importLocalBook,
  requestLinkImport: text => externalImports.requestLinkImport(text),
});

const externalImports = proxyRefs(useExternalImportActions({
  unlisteners,
  bootstrapped,
  getRestoreRevision: getFrontendRestoreRevision,
  getReaderSession,
  localBookImportBusy: localBookImportFeature.localBookImportBusy,
  sourceImportOpen,
  notify,
  errorText,
}));

function ignorePendingImportLink(): void {
  externalImports.pendingImportLinks.shift();
}

onMounted(() => {
  void (async () => {
    try {
      const unlistenAppState = await listen<AppBootstrap>("app-state-updated", (event) => refreshRestoredShelf(event.payload));
      if (disposed) unlistenAppState();
      else unlisteners.push(unlistenAppState);
      await localBookImportFeature.setupImports();
      void externalImports.setupImportLinks();
      const bootstrapRestoreRevision = getFrontendRestoreRevision();
      if (disposed || bootstrapRestoreRevision !== getFrontendRestoreRevision()) return;
      if (!disposed) bootstrapped.value = true;
    } catch (error) {
      if (!disposed) notify(`导入服务初始化失败：${errorText(error)}`, "error");
    }
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  externalImports.dispose();
  localBookImportFeature.dispose();
  removeLocalBookImportSession();
  for (const unlisten of unlisteners.splice(0)) unlisten();
});
</script>

<template>
  <KGlass component="aside" v-if="Object.keys(localBookImportFeature.importingFiles.value).length" class="file-import-progress material:bg-md-light-surface-2 material:dark:bg-md-dark-surface-2"
    :class="{ 'with-navigation': ['home', 'shelf', 'settings'].includes(String(router.currentRoute.value.name)) }" aria-live="polite">
    <div v-for="item in localBookImportFeature.importingFiles.value" :key="item.file" role="status">
      <span>{{ item.file }} · {{ stages[item.stage] }}<template v-if="item.total"> {{ Math.floor((item.completed ?? 0) * 100 / item.total) }}%</template></span>
      <KProgressbar v-if="item.total" :progress="(item.completed ?? 0) / item.total" class="mt-2" role="progressbar" aria-label="文件导入进度" :aria-valuenow="item.completed ?? 0" :aria-valuemax="item.total" :aria-valuemin="0" />
    </div>
  </KGlass>
  <ImportRequestBanners
    :pending-import-links="externalImports.pendingImportLinks"
    :import-link-opening="externalImports.importLinkOpening"
    :import-link-blocked="externalImports.importLinkBlocked"
    :recovery-required="shelfBatchRecoveryRequired"
    :source-import-open="sourceImportOpen || externalImports.importDialogOpen"
    :open-pending-import-link="externalImports.openPendingImportLink"
    :ignore-pending-import-link="ignorePendingImportLink"
  />
  <DeepLinkImportDialog
    :open="externalImports.importDialogOpen" :title="externalImports.importTitle"
    :busy="externalImports.importLinkOpening" :error="externalImports.importError"
    :entries="externalImports.importEntries" v-model:selected-keys="externalImports.selectedImportKeys"
    @save="externalImports.savePendingImport" @cancel="externalImports.cancelPendingImport()" @retry="externalImports.retryPendingImport"
  />
</template>

<style scoped>
@layer components {
.file-import-progress { position: fixed; z-index: 60; bottom: calc(16px + var(--safe-bottom) + var(--ui-playback-inset)); left: 50%; transform: translateX(-50%); width: min(420px, calc(100vw - 32px)); padding: 12px 16px; border-radius: 12px; }
.file-import-progress span { display: block; overflow-wrap: anywhere; }
@media (max-width: 860px) {
  .file-import-progress.with-navigation { bottom: calc(var(--ui-nav-height) + var(--safe-bottom) + var(--ui-playback-inset) + 16px); }
}
}
</style>
