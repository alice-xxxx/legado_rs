<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import type { AppBootstrap, ResourceDescriptor, ShelfResource } from "../../api/types";
import { notify } from "../../app/notifications";
import { getFrontendRestoreRevision, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { backupBusy } from "../settings/backupState";
import { getReaderSession } from "../reader/readerSession";
import { openedBook } from "../books/bookDetailState";
import { useLocalBookImport } from "../books/useLocalBookImport";
import PdfPasswordPrompt from "../books/PdfPasswordPrompt.vue";
import router, { routeNames } from "../../router";
import { useExternalImportActions } from "./useExternalImportActions";
import { installLocalBookImportSession } from "./localBookImportSession";
import ImportRequestBanners from "./ImportRequestBanners.vue";

type PdfPasswordPromptContext = {
  pdfPasswordPromptOpen: boolean;
  pdfPasswordInput: string;
  pdfPasswordBusy: boolean;
  pdfPasswordError: string;
  submitPdfPassword: () => void | Promise<void>;
  cancelPdfPasswordPrompt: () => void;
};

const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);
const bootstrapped = ref(false);
const externalFileRequestBusy = ref(false);
const shelf = ref<ShelfResource>({ schemaVersion: 1, groups: [], books: [] });
const pdfImportPasswords = new Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>();
const pendingPdfPassword = ref("");
const currentRoute = router.currentRoute;
const sourceImportOpen = computed(() => currentRoute.value.name === routeNames.sources
  && (currentRoute.value.query.sourceImport === "url" || currentRoute.value.query.sourceImport === "new"));
const unlisteners: UnlistenFn[] = [];
let disposed = false;
let shelfRequestRevision = 0;

async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++shelfRequestRevision;
  const currentRestoreRevision = getFrontendRestoreRevision();
  const next = await readResource<ShelfResource>(descriptor);
  if (disposed || requestRevision !== shelfRequestRevision || currentRestoreRevision !== getFrontendRestoreRevision()) return;
  shelf.value = { ...next, groups: next.groups ?? [], books: next.books ?? [] };
}

function refreshRestoredShelf(snapshot: AppBootstrap): void {
  void refreshShelf(snapshot.shelf).catch((error) => notify(`恢复后书架刷新失败：${errorText(error)}`, "error"));
  bootstrapped.value = true;
}

async function openSourceImport(url: string): Promise<void> {
  const failure = await router.push({ name: routeNames.sources, query: { sourceImport: "url", importUrl: url } });
  if (failure) throw new Error("无法打开书源导入页");
}

const localBookImportFeature = useLocalBookImport({
  openedBook,
  shelf,
  externalFileRequestBusy,
  backupBusy,
  recoveryRequired: shelfBatchRecoveryRequired,
  getRestoreRevision: getFrontendRestoreRevision,
  pdfImportPasswords,
  notify,
  errorText,
});
const removeLocalBookImportSession = installLocalBookImportSession({
  localBookImportBusy: localBookImportFeature.localBookImportBusy,
  pdfPasswordPromptOpen: localBookImportFeature.pdfPasswordPromptOpen,
  pdfPasswordBusy: localBookImportFeature.pdfPasswordBusy,
  pdfPasswordError: localBookImportFeature.pdfPasswordError,
  pdfImportPasswords,
  externalFileRequestBusy,
  backupBusy,
  importLocalBook: localBookImportFeature.importLocalBook,
  submitPdfPassword: localBookImportFeature.submitPdfPassword,
  cancelPdfPasswordPrompt: localBookImportFeature.cancelPdfPasswordPrompt,
});

const externalImports = proxyRefs(useExternalImportActions({
  unlisteners,
  bootstrapped,
  backupBusy,
  recoveryRequired: shelfBatchRecoveryRequired,
  getRestoreRevision: getFrontendRestoreRevision,
  getReaderSession,
  localBookImportBusy: localBookImportFeature.localBookImportBusy,
  externalFileRequestBusy,
  openPdfPasswordPrompt: localBookImportFeature.openPdfPasswordPrompt,
  acceptLocalBookImport: localBookImportFeature.acceptLocalBookImport,
  sourceImportOpen,
  openSourceImport,
  pdfPasswordPromptOpen: localBookImportFeature.pdfPasswordPromptOpen,
  notify,
  errorText,
}));

const pdfPasswordPromptContext: PdfPasswordPromptContext = {
  get pdfPasswordPromptOpen() { return localBookImportFeature.pdfPasswordPromptOpen.value; },
  get pdfPasswordInput() { return pendingPdfPassword.value; },
  set pdfPasswordInput(value: string) { pendingPdfPassword.value = value; },
  get pdfPasswordBusy() { return localBookImportFeature.pdfPasswordBusy.value; },
  get pdfPasswordError() { return localBookImportFeature.pdfPasswordError.value; },
  submitPdfPassword: () => {
    const password = pendingPdfPassword.value;
    pendingPdfPassword.value = "";
    if (password) void localBookImportFeature.submitPdfPassword(password);
  },
  cancelPdfPasswordPrompt: () => {
    pendingPdfPassword.value = "";
    void localBookImportFeature.cancelPdfPasswordPrompt();
  },
};

watch(localBookImportFeature.pdfPasswordPromptOpen, (open, wasOpen) => {
  if (open && !wasOpen) pendingPdfPassword.value = "";
});

function ignorePendingImportLink(): void {
  externalImports.pendingImportLinks.shift();
}

onMounted(() => {
  void (async () => {
    try {
      const unlistenShelf = await listen<ResourceDescriptor>("shelf-updated", (event) => {
        void refreshShelf(event.payload).catch((error) => notify(`书架刷新失败：${errorText(error)}`, "error"));
      });
      if (disposed) unlistenShelf();
      else unlisteners.push(unlistenShelf);
      const unlistenAppState = await listen<AppBootstrap>("app-state-updated", (event) => refreshRestoredShelf(event.payload));
      if (disposed) unlistenAppState();
      else unlisteners.push(unlistenAppState);
      void externalImports.setupExternalFiles();
      void externalImports.setupImportLinks();
      const bootstrapRestoreRevision = getFrontendRestoreRevision();
      const bootstrap = await appBootstrap();
      if (disposed || bootstrapRestoreRevision !== getFrontendRestoreRevision()) return;
      await refreshShelf(bootstrap.shelf);
      if (!disposed) bootstrapped.value = true;
    } catch (error) {
      if (!disposed) notify(`导入服务初始化失败：${errorText(error)}`, "error");
    }
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  shelfRequestRevision += 1;
  externalImports.dispose();
  localBookImportFeature.cancelPendingPdfImportOnUnmount();
  removeLocalBookImportSession();
  for (const entry of pdfImportPasswords.values()) clearTimeout(entry.timer);
  pdfImportPasswords.clear();
  for (const unlisten of unlisteners.splice(0)) unlisten();
});
</script>

<template>
  <ImportRequestBanners
    :pending-external-files="externalImports.pendingExternalFiles"
    :external-file-request-busy="externalImports.externalFileRequestBusy"
    :external-file-request-blocked="externalImports.externalFileRequestBlocked"
    :pending-import-links="externalImports.pendingImportLinks"
    :import-link-opening="externalImports.importLinkOpening"
    :import-link-blocked="externalImports.importLinkBlocked"
    :recovery-required="shelfBatchRecoveryRequired"
    :source-import-open="sourceImportOpen"
    :accept-external-file-request="externalImports.acceptExternalFileRequest"
    :ignore-external-file-request="externalImports.ignoreExternalFileRequest"
    :open-pending-import-link="externalImports.openPendingImportLink"
    :ignore-pending-import-link="ignorePendingImportLink"
  />
  <PdfPasswordPrompt :context="pdfPasswordPromptContext" />
</template>
