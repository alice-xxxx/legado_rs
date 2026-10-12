<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, proxyRefs,  shallowRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRoute } from "vue-router";
import type {
  AppBootstrap,
  AppTask,
  BookResource,
  ResourceDescriptor,
  TasksResource } from "../../api/types";
import { replacementRules } from "../settings/configurationStores";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { sources, bookSourceCandidates as sourceCandidates } from "../sources/sourcesStore";
import { refreshChapterContent } from "../../api/books";
import { readResource } from "../../api/resources";
import { startChapterDownload, tasksResource as loadTasksResource } from "../../api/tasks";
import { notify, errorText } from "../../app/notifications";
import {
  getFrontendRestoreRevision,
  lockShelfBatchRecovery,
  shelfBatchRecoveryMessage,
  shelfBatchRecoveryRequired,
  shelfBatchRecoveryTitle } from "../../app/recoveryState";
import { getLocalBookImportSession } from "../import/localBookImportSession";
import { backupBusy } from "../settings/backupState";
import { settings as sharedSettings } from "../settings/settingsStore";
import { clearOpenedBook, openedBook } from "../books/bookDetailState";
import { installReaderBackupSession } from "./readerBackupSession";
import { installReaderSession } from "./readerSession";
import { useAppReaderFeatures } from "./useAppReaderFeatures";
import { useBookSourceSwitch } from "../sources/useBookSourceSwitch";
import { installBookSourceSwitchSession } from "../sources/bookSourceSwitchSession";
import router, { routeNames } from "../../router";

const AppReader = defineAsyncComponent(() => import("./AppReader.vue"));
const route = useRoute();
const settings = sharedSettings;
watch(() => settings.value.reader.backgroundImageSrc, (next, previous) => {
  if (next !== previous) readerFeatures.rebuildReaderDisplay();
});


const tasks = shallowRef<AppTask[]>([]);

let tasksDescriptor: ResourceDescriptor | null = null;
let disposed = false;
let tasksReadRevision = 0;
let readerDataRefreshPending = false;
const unlisteners: UnlistenFn[] = [];
const readerImportPasswords = getLocalBookImportSession().pdfImportPasswords;

async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const restoreRevision = getFrontendRestoreRevision();
  const requestRevision = ++tasksReadRevision;
  if (descriptor) tasksDescriptor = descriptor;
  if (!tasksDescriptor) tasksDescriptor = (await loadTasksResource()).resource;
  const document = await readResource<TasksResource>(tasksDescriptor);
  if (restoreRevision !== getFrontendRestoreRevision() || requestRevision !== tasksReadRevision) return;
  const byId = new Map((document.tasks ?? []).map((task) => [task.id, task]));
  for (const task of tasks.value) if (task.status === "recoveryRequired") byId.set(task.id, task);
  tasks.value = [...byId.values()];
}

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
  settings,
  replacementRules,
  openedBook,
  shelfBatchRecoveryRequired,
  backupBusy,
  pdfImportPasswords: readerImportPasswords,
  getRestoreRevision: getFrontendRestoreRevision,
  notify,
  errorText,
  lockChapterCacheRecovery: lockReaderRecovery,
  refreshChapterContent,
  startChapterDownload,
  refreshTasks,
  reusedCatalogTaskNotice,
  openOtherSettings: () => {
    void router.push({ name: routeNames.settingsOther });
  },
  showShelfBehindReader,
  readResource,
});

const bookSourceSwitchActions = useBookSourceSwitch({
  bookSourceCandidates: sourceCandidates,
  tasks,
  openedBook,
  readingBook: readerFeatures.readingBook,
  readerVisible: readerFeatures.readerVisible,
  readingChapterIndex: readerFeatures.readingChapterIndex,
  readerPageIndex: readerFeatures.readerPageIndex,
  readingChapterHtml: readerFeatures.readingChapterHtml,
  readingChapterRaw: readerFeatures.readingChapterRaw,
  readingPdfPage: readerFeatures.readingPdfPage,
  readingImagePage: readerFeatures.readingImagePage,
  readerProgressDirty: readerFeatures.readerProgressDirty,
  readerControlsOpen: readerFeatures.readerControlsOpen,
  shelf,
  bookmarks: readerFeatures.bookmarks,
  shelfBatchRecoveryRequired,
  shelfBatchRecoveryTitle,
  shelfBatchRecoveryMessage,
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  refreshTasks: () => refreshTasks(),
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
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  finishReadingSession: readerFeatures.finishReadingSession,
  closeReader: closeReaderForNavigation,
  openBookmark: readerFeatures.openBookmark,
});
const disposeBackupSession = installReaderBackupSession({
  settings,
  readingBook: readerFeatures.readingBook,
  readerProgressDirty: readerFeatures.readerProgressDirty,
  hasActiveReaderMedia: readerFeatures.hasActiveReaderMedia,
  pauseReaderMediaAndSave: readerFeatures.pauseReaderMediaAndSave,
  saveCurrentProgress: readerFeatures.saveCurrentProgress,
  closeReaderAfterRestore: readerFeatures.closeReaderAfterRestore,
});
const disposeBookSourceSwitch = installBookSourceSwitchSession(bookSourceSwitch);

async function bootstrap(): Promise<void> {
  const restoreRevision = getFrontendRestoreRevision();

  const tasksRequestRevision = ++tasksReadRevision;
  const taskDescriptor = await loadTasksResource();
  if (restoreRevision !== getFrontendRestoreRevision()) return;
  const nextTasks = await readResource<TasksResource>(taskDescriptor.resource);
  if (restoreRevision !== getFrontendRestoreRevision()) return;
  if (tasksRequestRevision === tasksReadRevision) {
    tasksDescriptor = taskDescriptor.resource;
    tasks.value = nextTasks.tasks ?? [];
  }
  if (backupBusy.value === "restore") {
    readerDataRefreshPending = true;
  } else {
    readerDataRefreshPending = false;
    await readerFeatures.refreshBookmarksAndReadingHistory();
  }
}

watch(backupBusy, (busy) => {
  if (busy !== "restore" && readerDataRefreshPending) {
    readerDataRefreshPending = false;
    void readerFeatures.refreshBookmarksAndReadingHistory().catch((error) => {
      notify(`恢复后的阅读记录载入失败：${errorText(error)}`, "error");
    });
  }
});

function retain(unlisten: UnlistenFn): void {
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}

async function setupReaderEvents(): Promise<void> {
  try {
    retain(await listen<{ requestId: number; resource: ResourceDescriptor }>("history-updated", (event) => {
      void readerFeatures.readerBookmarksAndHistory.refreshCurrentReadingStatistics();
      if (event.payload.requestId === 0) {
        void readerFeatures.readerBookmarksAndHistory.refreshBookmarksAfterHistoryUpdate();
      }
    }));
    retain(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", async (event) => {
      const revision = getFrontendRestoreRevision();
      try {
        if (event.payload.kind === "tasks") {
          await refreshTasks(event.payload.resource);
          await bookSourceSwitchActions.syncBookSourceCandidateTask();

        }
      } catch (error) {
        if (revision === getFrontendRestoreRevision()) notify(`阅读器数据更新失败：${errorText(error)}`, "error");
      }
    }));
    retain(await listen<{ taskId: string; resource: ResourceDescriptor }>("task-updated", async (event) => {
      const revision = getFrontendRestoreRevision();
      try {
        await refreshTasks(event.payload.resource);
        if (revision === getFrontendRestoreRevision()) {
          const task = tasks.value.find((item) => item.id === event.payload.taskId);
          await bookSourceSwitchActions.syncBookSourceCandidateTask(task);
        }
      } catch (error) {
        if (revision === getFrontendRestoreRevision()) notify(`候选搜索状态更新失败：${errorText(error)}`, "error");
      }
    }));
    retain(await listen<Partial<AppTask>>("task-recovery-required", async (event) => {
      await bookSourceSwitchActions.syncBookSourceCandidateTask(event.payload);
    }));
    retain(await listen<AppBootstrap>("app-state-updated", async () => {
      const revision = getFrontendRestoreRevision();
      tasks.value = [];
      tasksDescriptor = null;
      bookSourceSwitchActions.resetForRestore();
      try {
        await bootstrap();
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
  if (!readerFeatures.readerVisible.value || readerFeatures.shouldIgnoreReaderKeydown(event, document)) return;
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
  void bootstrap().catch((error) => notify(`阅读器初始化失败：${errorText(error)}`, "error"));
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

const bookSourceCandidates = sourceCandidates;
const readerVisible = readerFeatures.readerVisible;
</script>

<template>
  <AppReader
    v-if="readerVisible"
    :reader-features="readerFeatures"
    :close-reader="closeReaderForNavigation"
    :book-source-switch="bookSourceSwitch"
    :book-source-candidates="bookSourceCandidates"
    :source-name="sourceName"
    :shelf-batch-recovery-required="shelfBatchRecoveryRequired"
  />
</template>
