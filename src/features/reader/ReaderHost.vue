<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, proxyRefs, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { setSystemUiHidden } from "tauri-plugin-screen-api";
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
import { installReaderNativeBack, localBackAction } from "../../app/nativeBack";
import { settings } from "../settings/settingsStore";
import { isVerticalReadingMode } from "../settings/readerModes";
import PlaybackBar from "../../ui/PlaybackBar.vue";
import { readBook } from "../../api/database";
import { refreshReadingChapter } from "./readerBookState";

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
  showShelfBehindReader,
});
const player = proxyRefs(readerFeatures);
const readerPresented = computed(() => player.readerVisible && !player.readerMinimized);
const audioSession = computed(() => player.readingMediaChapter);
const showPlaybackBar = computed(() => player.readerVisible && (player.ttsActive || audioSession.value && player.readerMinimized)
  && !player.readerControlsOpen && !player.readerDirectoryOpen && !player.readerTtsSettingsOpen && !player.readerOverlayOpen);
watch(() => showPlaybackBar.value && player.readerMinimized, visible => {
  document.body.classList.toggle("has-global-playback", visible);
}, { immediate: true });
const playbackPreparing = computed(() => audioSession.value ? player.readerBusy || player.readerMediaBuffering
  : !player.ttsChunkProgress || player.ttsBuffering && !player.ttsPaused);
const playbackPaused = computed(() => audioSession.value ? !player.readerMediaPlaying : player.ttsPaused);
const playbackStatus = computed(() => playbackPreparing.value ? "正在准备播放…" : audioSession.value && player.readerMediaEnded ? "播放结束" : playbackPaused.value ? "已暂停"
  : audioSession.value ? "正在播放" : "正在朗读");
async function expandPlayback(): Promise<void> {
  if (player.readerMinimized) {
    player.readerMinimized = false;
    await nextTick();
    readerFeatures.handleReaderResize();
  } else readerFeatures.openTtsControls();
}
function togglePlayback(): void {
  if (audioSession.value) readerFeatures.toggleReaderMediaPlayback();
  else void readerFeatures.toggleTtsPaused();
}
async function stopPlayback(): Promise<void> {
  if (player.readerMinimized) await closeReaderForNavigation();
  else await readerFeatures.stopTts();
}
async function leaveReader(): Promise<void> {
  if (player.canMinimizeReader) readerFeatures.minimizeReader();
  else await closeReaderForNavigation();
}
watch(readerFeatures.ttsActive, active => {
  if (!active && player.readerMinimized && !audioSession.value) void closeReaderForNavigation();
});

const bookSourceSwitchActions = useBookSourceSwitch({
  readingBook: readerFeatures.readingBook,
  readerVisible: readerFeatures.readerVisible,
  readingChapterIndex: readerFeatures.readingChapterIndex,
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
watch(() => bookSourceSwitch.bookSourceSwitchOpen, open => {
  readerFeatures.readerOverlayOpen.value = open;
  if (open) {
    readerFeatures.readerControlsOpen.value = false;
    readerFeatures.readerDirectoryOpen.value = false;
  }
}, { flush: "sync", immediate: true });
watch(() => readerFeatures.readerControlsOpen.value || readerFeatures.readerDirectoryOpen.value
  || readerFeatures.readerTtsSettingsOpen.value || readerFeatures.readerOverlayOpen.value, open => {
  if (open) readerFeatures.readerMenuOpen.value = true;
}, { flush: "sync" });
let closingReader = false;
async function closeReaderForNavigation(discardProgress = false): Promise<void> {
  if (closingReader) return;
  closingReader = true;
  try {
    await readerFeatures.closeReader(discardProgress);
    if (!readerFeatures.readerVisible.value && route.meta.screen !== "detail") clearOpenedBook();
    else if (readerFeatures.readerVisible.value) player.readerMinimized = false;
  } finally { closingReader = false; }
}

const disposeReaderSession = installReaderSession({
  readerVisible: readerPresented,
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
  visible: readerPresented,
  menuOpen: computed(() => readerFeatures.readerMenuOpen.value || readerFeatures.readingMediaChapter.value
    || readerFeatures.readerBusy.value || !!readerFeatures.readerChapterError.value),
  close: async () => {
    if (!readerFeatures.readerMediaRetrying.value) await leaveReader();
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
    retain(await listen<{ bookId: string; chapterId?: string }>("book-updated", ({ payload }) => {
      if (player.readingBook?.id !== payload.bookId) return;
      if (payload.chapterId) void refreshReadingChapter(payload.bookId, payload.chapterId).catch(error => notify(errorText(error), "error"));
      else void readBook(payload.bookId).then(book => {
        if (player.readingBook?.id === book.id) readerFeatures.applyBookInformation(book);
      }).catch(error => notify(errorText(error), "error"));
    }));
    retain(await listen<{ bookId: string; progress: NonNullable<BookResource["progress"]> }>("reading-progress-updated", ({ payload }) => {
      const book = player.readingBook;
      if (book?.id === payload.bookId && (book.progress?.updatedAtMs ?? 0) <= (payload.progress.updatedAtMs ?? 0)) book.progress = payload.progress;
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
  if (!readerPresented.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    void localBackAction()?.run();
    return;
  }
  if (readerFeatures.readerGesturesBlocked.value) return;
  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "b") {
    const target = event.target as HTMLElement | null;
    if (target?.closest("input,textarea,select,[contenteditable]:not([contenteditable='false'])")) return;
    event.preventDefault();
    if (!event.repeat && !readerFeatures.readerBusy.value) void readerFeatures.toggleCurrentBookmark();
    return;
  }
  if (readerFeatures.shouldIgnoreReaderKeydown(event, document)) return;
  if (readerFeatures.readingMediaChapter.value) {
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
  }
}

function onVisibilityChange(): void {
  if (document.visibilityState === "hidden") {
    if (readerFeatures.readingVideoChapter.value && readerFeatures.getReaderMediaElement()) void readerFeatures.pauseReaderMediaAndSave();
    else void readerFeatures.saveCurrentProgress();
    void readerFeatures.finishReadingSession();
  } else if (readerFeatures.readerVisible.value) {
    syncReaderFullscreen(readerPresented.value);
    readerFeatures.handleReaderResize();
    void readerFeatures.saveCurrentProgress();
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
  if (next !== previous && readerPresented.value) void leaveReader();
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
    if (disposed || readerPresented.value !== visible) return;
    try {
      await setSystemUiHidden(visible);
    } catch (error) {
      console.warn("Reader fullscreen could not be changed:", error);
    }
  });
}
watch(readerPresented, syncReaderFullscreen);
watch(() => settings.value.uiStyle, async () => {
  await nextTick();
  if (readerPresented.value) readerFeatures.handleReaderResize();
});
onBeforeUnmount(() => {
  document.body.classList.remove("has-global-playback");
  if (fullscreenUsed) void fullscreenUpdate.catch(() => {}).then(() => setSystemUiHidden(false)).catch(() => {});
});
</script>

<template>
  <div v-if="readerVisible" class="reader-view" :class="{ minimized: player.readerMinimized }"
    :inert="player.readerMinimized" :aria-hidden="player.readerMinimized">
  <component :is="readerFeatures.readingPdfPage.value ? PdfReader : readerFeatures.readingVideoChapter.value ? VideoReader : readerFeatures.readingMediaChapter.value ? AudioReader : readerFeatures.readingImagePage.value ? ComicReader : readerFeatures.readingContentType.value === 'html' ? HtmlReader : NovelReader"
    :reader-features="readerFeatures"
    :close-reader="closeReaderForNavigation"
    :book-source-switch="bookSourceSwitch"
    :shelf-batch-recovery-required="shelfBatchRecoveryRequired"
  />
  </div>
  <PlaybackBar v-if="showPlaybackBar" :title="player.readingBook?.title ?? ''" :subtitle="player.currentChapterTitle"
    :icon="player.readingVideoChapter ? 'video' : audioSession ? 'audio' : 'tts'" :paused="playbackPaused" :preparing="playbackPreparing" :status="playbackStatus"
    :disabled="audioSession ? player.isReaderMediaControlBlocked() : !player.ttsChunkProgress"
    :error="audioSession ? player.readerMediaError : player.ttsError"
    :menu-open="!player.readerMinimized && player.readerMenuOpen" :global="player.readerMinimized"
    :navigation="route.name === routeNames.home || route.name === routeNames.shelf || route.name === routeNames.settings"
    @expand="expandPlayback" @toggle="togglePlayback" @stop="stopPlayback" />
</template>

<style scoped>
@layer components {
.reader-view { display: flex; position: absolute; inset: 0; }
/* Keep the text layout and audio element alive while the rest of the app is usable. */
.reader-view.minimized { transform: translateX(-200vw); visibility: hidden; pointer-events: none; }
}
</style>
