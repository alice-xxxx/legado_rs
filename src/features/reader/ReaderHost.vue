<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, proxyRefs, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useRoute } from "vue-router";
import type {
  AppBootstrap,
  BookResource } from "../../api/types";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { notify, errorText } from "../../app/notifications";
import {
  getFrontendRestoreRevision,
  lockShelfBatchRecovery,
  shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { clearOpenedBook } from "../books/bookDetailState";
import { installReaderBackupSession } from "./readerBackupSession";
import { installReaderSession } from "./readerSession";
import { useAppReaderFeatures } from "./useAppReaderFeatures";
import { useBookSourceSwitch } from "../sources/useBookSourceSwitch";
import { installBookSourceSwitchSession } from "../sources/bookSourceSwitchSession";
import router, { routeNames } from "../../router";
import { installReaderNativeBack } from "../../app/nativeBack";
import { refreshNativeAudio, disposeNativeAudio } from "./nativeAudio";
import { settings } from "../settings/settingsStore";
import { isVerticalReadingMode } from "../settings/readerModes";

const HtmlReader = defineAsyncComponent(() => import("./HtmlReader.vue"));
const NovelReader = defineAsyncComponent(() => import("./NovelReader.vue"));
const ComicReader = defineAsyncComponent(() => import("./ComicReader.vue"));
const AudioReader = defineAsyncComponent(() => import("./AudioReader.vue"));
const VideoReader = defineAsyncComponent(() => import("./VideoReader.vue"));
const PdfReader = defineAsyncComponent(() => import("./PdfReader.vue"));
const route = useRoute();


let disposed = false;
const unlisteners: UnlistenFn[] = [];

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
  getRestoreRevision: getFrontendRestoreRevision,
  notify,
  errorText,
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
  applyBookInformation: readerFeatures.applyBookInformation,
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
const disposeNativeBack = installReaderNativeBack({
  visible: readerFeatures.readerVisible,
  close: async () => {
    if (!readerFeatures.readerMediaRetrying.value) await closeReaderForNavigation();
  },
});

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
    retain(await listen<AppBootstrap>("app-state-updated", () => {
      bookSourceSwitchActions.resetForRestore();
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
  if (bookSourceSwitch.bookSourceSwitchOpen || readerFeatures.readerDirectoryOpen.value) return;
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
  const verticalText = !readerFeatures.readingImagePage.value && !readerFeatures.readingMediaChapter.value
    && !readerFeatures.readingPdfPage.value && isVerticalReadingMode(settings.value.reader.readingMode);
  if (verticalText && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
    event.preventDefault();
    return;
  }
  if (verticalText && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
    event.preventDefault();
    const direction = event.key === "ArrowDown" ? 1 : -1;
    const viewport = readerFeatures.readerViewport.value;
    if (settings.value.reader.readingMode === "vertical" && viewport
      && (direction > 0 ? viewport.scrollTop + viewport.clientHeight < viewport.scrollHeight - 1 : viewport.scrollTop > 1)) {
      viewport.scrollBy({ top: direction * 40, behavior: "instant" as ScrollBehavior });
    } else readerFeatures.turnPage(direction);
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
    if (readerFeatures.readingVideoChapter.value && readerFeatures.getReaderMediaElement()) void readerFeatures.pauseReaderMediaAndSave();
    else void readerFeatures.saveCurrentProgress();
    void readerFeatures.finishReadingSession();
  } else if (readerFeatures.readerVisible.value) {
    syncReaderFullscreen(true);
    readerFeatures.handleReaderResize();
    void refreshNativeAudio().then(() => readerFeatures.saveCurrentProgress())
      .catch(error => notify(errorText(error), "error"));
    void readerFeatures.beginReadingSession();
  }
}

function onPageHide(): void {
  if (readerFeatures.readingVideoChapter.value && readerFeatures.getReaderMediaElement()) void readerFeatures.pauseReaderMediaAndSave();
  else void readerFeatures.saveCurrentProgress();
  void readerFeatures.finishReadingSession();
}

const screen = computed(() => route.meta.screen ?? "home");
watch(screen, (next, previous) => {
  if (next !== previous && readerFeatures.readerVisible.value) void closeReaderForNavigation();
});

onMounted(() => {
  void readerFeatures.setupTts().catch(error => notify(errorText(error), "error"));
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
  disposeBookSourceSwitch();
  disposeBackupSession();
  disposeReaderSession();
  disposeNativeBack();
  void disposeNativeAudio().catch(error => console.debug("Native audio disposal failed:", error));
});

const readerVisible = readerFeatures.readerVisible;
let fullscreenUpdate = Promise.resolve();
const nativeFullscreen = isTauri() && (/iPhone|iPad|iPod/.test(navigator.userAgent)
  || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
let fullscreenUsed = false;
function syncReaderFullscreen(visible: boolean): void {
  if (!nativeFullscreen) return;
  fullscreenUsed = true;
  fullscreenUpdate = fullscreenUpdate.catch(() => {}).then(async () => {
    if (disposed || readerVisible.value !== visible) return;
    try {
      await invoke<boolean>("set_reader_fullscreen", { hidden: visible });
    } catch (error) {
      console.warn("Reader fullscreen could not be changed:", error);
    }
  });
}
watch(readerVisible, syncReaderFullscreen);
onBeforeUnmount(() => {
  if (fullscreenUsed) void fullscreenUpdate.catch(() => {}).then(() => invoke("set_reader_fullscreen", { hidden: false })).catch(() => {});
});
</script>

<template>
  <component :is="readerFeatures.readingPdfPage.value ? PdfReader : readerFeatures.readingVideoChapter.value ? VideoReader : readerFeatures.readingMediaChapter.value ? AudioReader : readerFeatures.readingImagePage.value ? ComicReader : readerFeatures.readingContentType.value === 'html' ? HtmlReader : NovelReader"
    v-if="readerVisible"
    :reader-features="readerFeatures"
    :close-reader="closeReaderForNavigation"
    :book-source-switch="bookSourceSwitch"
    :shelf-batch-recovery-required="shelfBatchRecoveryRequired"
  />
</template>
