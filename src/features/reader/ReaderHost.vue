<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, proxyRefs, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRoute } from "vue-router";
import type {
  AppBootstrap,
  AppTask,
  BookResource } from "../../api/types";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { sources } from "../sources/sourcesStore";
import { startChapterDownload } from "../../api/tasks";
import { notify, errorText } from "../../app/notifications";
import { tasks, latestTaskUpdate, ensureTasksLoaded } from "../tasks/tasksStore";
import {
  getFrontendRestoreRevision,
  lockShelfBatchRecovery,
  shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { getLocalBookImportSession } from "../import/localBookImportSession";
import { clearOpenedBook } from "../books/bookDetailState";
import { installReaderBackupSession } from "./readerBackupSession";
import { installReaderSession } from "./readerSession";
import { useAppReaderFeatures } from "./useAppReaderFeatures";
import { useBookSourceSwitch } from "../sources/useBookSourceSwitch";
import { installBookSourceSwitchSession } from "../sources/bookSourceSwitchSession";
import router, { routeNames } from "../../router";

const HtmlReader = defineAsyncComponent(() => import("./HtmlReader.vue"));
const NovelReader = defineAsyncComponent(() => import("./NovelReader.vue"));
const ComicReader = defineAsyncComponent(() => import("./ComicReader.vue"));
const AudioReader = defineAsyncComponent(() => import("./AudioReader.vue"));
const VideoReader = defineAsyncComponent(() => import("./VideoReader.vue"));
const PdfReader = defineAsyncComponent(() => import("./PdfReader.vue"));
const route = useRoute();


let disposed = false;
const unlisteners: UnlistenFn[] = [];
const readerImportPasswords = getLocalBookImportSession().pdfImportPasswords;

function reusedCatalogTaskNotice(task: AppTask): string {
  const status = task.status === "paused" ? "已暂停" : task.status === "interrupted" ? "已中断"
    : task.status === "queued" ? "排队中" : task.status === "cancelling" ? "正在取消"
      : task.status === "pausing" ? "正在暂停" : task.status === "recoveryRequired" ? "需要恢复" : "正在运行";
  return `已有全书章节缓存任务（${status}），已复用当前任务。`;
}

function sourceName(id: string): string {
  return sources.value.find((source) => source.id === id)?.name ?? id;
}

async function showShelfBehindReader(book: BookResource): Promise<void> {
  const currentScreen = route.meta.screen;
  if (shelf.value.books.some((entry) => entry.id === book.id)
    && currentScreen !== "shelf" && currentScreen !== "detail") {
    await router.push({ name: routeNames.shelf });
  }
}

function lockReaderRecovery(title: string, message: string): void {
  lockShelfBatchRecovery(title, message);
}

const readerFeatures = useAppReaderFeatures({
  pdfImportPasswords: readerImportPasswords,
  getRestoreRevision: getFrontendRestoreRevision,
  notify,
  errorText,
  startChapterDownload,
  reusedCatalogTaskNotice,
  openOtherSettings: () => {
    void router.push({ name: routeNames.settingsOther });
  },
  showShelfBehindReader,
});

const bookSourceSwitchActions = useBookSourceSwitch({
  readingBook: readerFeatures.readingBook,
  readerVisible: readerFeatures.readerVisible,
  readingChapterIndex: readerFeatures.readingChapterIndex,
  readerPageIndex: readerFeatures.readerPageIndex,
  readingContent: readerFeatures.readingContent,
  readerProgressDirty: readerFeatures.readerProgressDirty,
  readerControlsOpen: readerFeatures.readerControlsOpen,
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  loadReaderChapter: readerFeatures.loadReaderChapter,
  notify,
  errorText,
  getRestoreRevision: getFrontendRestoreRevision,
});
const bookSourceSwitch = proxyRefs(bookSourceSwitchActions);
watch(tasks, () => {
  void bookSourceSwitchActions.syncBookSourceCandidateTask().catch(error => notify(errorText(error), "error"));
});
watch(latestTaskUpdate, task => {
  if (task) void bookSourceSwitchActions.syncBookSourceCandidateTask(task).catch(error => notify(errorText(error), "error"));
}, { flush: "sync" });


async function closeReaderForNavigation(discardProgress = false): Promise<void> {
  await readerFeatures.closeReader(discardProgress);
  if (!readerFeatures.readerVisible.value && route.meta.screen !== "detail") clearOpenedBook();
}

const disposeReaderSession = installReaderSession({
  readerVisible: readerFeatures.readerVisible,
  readerBusy: readerFeatures.readerBusy,
  readingBook: readerFeatures.readingBook,
  readerProgressDirty: readerFeatures.readerProgressDirty,
  startReading: readerFeatures.startReading,
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  finishReadingSession: readerFeatures.finishReadingSession,
  closeReader: closeReaderForNavigation,
  openBookmark: readerFeatures.openBookmark,
});
const disposeBackupSession = installReaderBackupSession({
  readingBook: readerFeatures.readingBook,
  readerProgressDirty: readerFeatures.readerProgressDirty,
  hasActiveReaderMedia: readerFeatures.hasActiveReaderMedia,
  pauseReaderMediaAndSave: readerFeatures.pauseReaderMediaAndSave,
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  closeReaderAfterRestore: readerFeatures.closeReaderAfterRestore,
});
const disposeBookSourceSwitch = installBookSourceSwitchSession(bookSourceSwitch);

function retain(unlisten: UnlistenFn): void {
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}

async function setupReaderEvents(): Promise<void> {
  try {
    retain(await listen<{ title: string; message: string }>("reader-recovery-required", ({ payload }) => {
      lockReaderRecovery(payload.title, payload.message);
    }));
    retain(await listen<{ bookId: string; chapterIndex: number; error: string }>("reading-resource-error", ({ payload }) => {
      if (!readerFeatures.readerVisible.value || readerFeatures.readingBook.value?.id !== payload.bookId) return;
      if (readerFeatures.readerBusy.value && readerFeatures.readingChapterIndex.value === payload.chapterIndex) readerFeatures.readerChapterLoading.failPending(payload.error);
      else notify(payload.error, "error");
    }));
    retain(await listen<AppBootstrap>("app-state-updated", async () => {
      const revision = getFrontendRestoreRevision();
      bookSourceSwitchActions.resetForRestore();
      try {
        await ensureTasksLoaded();
        if (revision !== getFrontendRestoreRevision()) return;
      } catch (error) {
        if (revision !== getFrontendRestoreRevision()) return;
        const detail = errorText(error);
        lockShelfBatchRecovery("恢复后的数据载入失败", `已恢复的数据未能完整载入：${detail}。请重启应用后再继续。`);
        try { await readerFeatures.closeReaderAfterRestore(); } catch { /* Keep recovery lock. */ }
        notify(`恢复后的阅读数据重新载入失败：${detail}；请重启应用。`, "error");
      }
    }));
  } catch (error) {
    if (!disposed) console.debug("Reader event subscription unavailable:", errorText(error));
  }
}

function onGlobalKeydown(event: KeyboardEvent): void {
  if (event.defaultPrevented || event.isComposing) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    void router.push({ name: routeNames.search });
    return;
  }
  if (!readerFeatures.readerVisible.value) return;
  if (event.key === "Escape") {
    if (readerFeatures.bookmarkEditorOpen.value) {
      event.preventDefault();
      readerFeatures.closeBookmarkEditor();
      return;
    }
    if (readerFeatures.dictionaryOpen.value) {
      event.preventDefault();
      readerFeatures.closeReaderDictionary();
      return;
    }
    if (bookSourceSwitch.bookSourceSwitchOpen) {
      event.preventDefault();
      bookSourceSwitch.closeBookSourceSwitch();
      return;
    }
    if (readerFeatures.readerDirectoryOpen.value || readerFeatures.readerControlsOpen.value) {
      event.preventDefault();
      readerFeatures.readerDirectoryOpen.value = false;
      readerFeatures.readerControlsOpen.value = false;
      return;
    }
  }
  if (readerFeatures.shouldIgnoreReaderKeydown(event, document)) return;
  if (readerFeatures.readingMediaChapter.value && event.key !== "Escape") {
    if (event.key === " " || event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      if (readerFeatures.isReaderMediaControlBlocked()) return;
      if (event.key === " ") readerFeatures.toggleReaderMediaPlayback();
      else readerFeatures.skipReaderMedia(event.key === "ArrowLeft" ? -10 : 10);
    }
    return;
  }
  if (event.key === "ArrowRight" || event.key === "PageDown" || event.key === " ") {
    event.preventDefault();
    readerFeatures.turnPage(1);
  } else if (event.key === "ArrowLeft" || event.key === "PageUp") {
    event.preventDefault();
    readerFeatures.turnPage(-1);
  } else if (event.key === "Escape") {
    event.preventDefault();
    void closeReaderForNavigation();
  }
}

function onVisibilityChange(): void {
  if (document.visibilityState === "hidden") {
    readerFeatures.stopHttpTtsAudio(false);
    if (readerFeatures.getReaderMediaElement()) void readerFeatures.pauseReaderMediaAndSave();
    else void readerFeatures.saveCurrentProgress();
    void readerFeatures.finishReadingSession();
  } else if (readerFeatures.readerVisible.value) {
    void readerFeatures.beginReadingSession();
  }
}

function onPageHide(): void {
  readerFeatures.stopHttpTtsAudio(false);
  if (readerFeatures.getReaderMediaElement()) void readerFeatures.pauseReaderMediaAndSave();
  else void readerFeatures.saveCurrentProgress();
  void readerFeatures.finishReadingSession();
}

const screen = computed(() => route.meta.screen ?? "home");
watch(screen, (next, previous) => {
  if (next !== previous && readerFeatures.readerVisible.value) void closeReaderForNavigation();
});

onMounted(() => {
  readerFeatures.setupTts();
  void setupReaderEvents();
  window.addEventListener("keydown", onGlobalKeydown);
  window.addEventListener("pagehide", onPageHide);
  window.addEventListener("resize", readerFeatures.handleReaderResize);
  document.addEventListener("visibilitychange", onVisibilityChange);
});

onBeforeUnmount(() => {
  disposed = true;
  window.removeEventListener("keydown", onGlobalKeydown);
  window.removeEventListener("pagehide", onPageHide);
  window.removeEventListener("resize", readerFeatures.handleReaderResize);
  document.removeEventListener("visibilitychange", onVisibilityChange);
  for (const unlisten of unlisteners.splice(0)) unlisten();
  readerFeatures.clearReaderMediaSleepTimer();
  if (readerFeatures.getReaderMediaElement()) {
    const media = readerFeatures.getReaderMediaElement()!;
    void readerFeatures.pauseReaderMediaAndSave();
    readerFeatures.clearReaderMediaBinding(false);
    media.pause();
  }
  readerFeatures.clearReaderScrollListeners();
  void readerFeatures.saveCurrentProgress();
  void readerFeatures.finishReadingSession();
  readerFeatures.cancelPendingPdfImportOnUnmount();
  disposeBookSourceSwitch();
  disposeBackupSession();
  disposeReaderSession();
});

const bookSourceCandidates = computed(() => bookSourceSwitch.bookSourceEligibleSources);
const readerVisible = readerFeatures.readerVisible;
</script>

<template>
  <component :is="readerFeatures.readingPdfPage.value ? PdfReader : readerFeatures.readingVideoChapter.value ? VideoReader : readerFeatures.readingMediaChapter.value ? AudioReader : readerFeatures.readingImagePage.value ? ComicReader : readerFeatures.readingContentType.value === 'html' ? HtmlReader : NovelReader"
    v-if="readerVisible"
    :reader-features="readerFeatures"
    :close-reader="closeReaderForNavigation"
    :book-source-switch="bookSourceSwitch"
    :book-source-candidates="bookSourceCandidates"
    :source-name="sourceName"
    :shelf-batch-recovery-required="shelfBatchRecoveryRequired"
  />
</template>
