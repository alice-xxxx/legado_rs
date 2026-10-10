<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { clearBookChapterCache as clearBookChapterCacheApi, getBook } from "../../api/books";
import { readResource } from "../../api/resources";
import { checkNewChapters, refreshChapters, startBookDownload, tasksResource } from "../../api/tasks";
import type { AppBootstrap, AppTask, BookResource, ResourceDescriptor, ShelfResource, SourceDefinitionsResource, SourceMetadata, TasksResource } from "../../api/types";
import router, { routeNames } from "../../router";
import BackButton from "../../ui/BackButton.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BookDisplayMetadataEditor from "./BookDisplayMetadataEditor.vue";
import { useBookDetailActions } from "./useBookDetailActions";
import { useBookMetadataActions } from "./useBookMetadataActions";
import { hasReadingProgress, readingProgressPercent } from "./readingProgress";
import { getReaderSession } from "../reader/readerSession";
import { getBookSourceSwitchSession } from "../sources/bookSourceSwitchSession";
import BookSourceSwitchSheet from "../sources/BookSourceSwitchSheet.vue";
import { consumeBookDetailOpenRequest } from "./bookDetailOpenRequest";
import { useBookDetailSession } from "./useBookDetailSession";
import { openedBook } from "./bookDetailState";
import { errorText, notify } from "../../app/notifications";
import { getFrontendRestoreRevision, lockShelfBatchRecovery, shelfBatchRecoveryMessage, shelfBatchRecoveryRequired, shelfBatchRecoveryTitle } from "../../app/recoveryState";

type BookTaskKind = "refreshChapters" | "checkNewChapters";
type BookTaskAction = "download" | "refresh" | "check";
const shelf = shallowRef<ShelfResource>({ schemaVersion: 1, books: [] });
const sources = shallowRef<SourceMetadata[]>([]);
const bookSourceCandidates = computed(() => sources.value.filter((source) => source.enabled && source.isRss !== true));
const tasks = shallowRef<AppTask[]>([]);
const sourceName = (id: string): string => sources.value.find((source) => source.id === id)?.name ?? id;
const readerSession = getReaderSession();
const { readerVisible, readingBook } = readerSession;
const bookSourceSwitch = getBookSourceSwitchSession();
const recoveryRequired = shelfBatchRecoveryRequired;
const openRequest = consumeBookDetailOpenRequest();
const bookPanelBusy = ref(Boolean(openRequest));
const pageResourceLoading = ref(false);
const pageResourceError = ref("");
const bookInfoRefreshBusy = ref(false);
const bookProgressResetBusy = ref(false);
const bookProgressResetError = ref("");
const pendingBookProgressReset = ref<{ id: string; title: string } | null>(null);
const bookDisplayEditorBookId = ref<string | null>(null);
const bookDisplayEditorError = ref("");
const bookDisplaySaveBusy = ref(false);
const pendingRemoval = ref<{ id: string; title: string } | null>(null);
const singleBookRemovalBusy = ref(false);
const bookChapterCacheClearBusy = ref(false);
const bookChapterCacheClearError = ref("");
const pendingBookChapterCacheClear = ref<{ id: string; title: string } | null>(null);
const detailGlobalError = ref("");
let sourceDefinitionsResource: ResourceDescriptor | null = null;
let shelfReadRevision = 0;
let sourcesReadRevision = 0;
let tasksReadRevision = 0;
let pageResourceRevision = 0;
let disposed = false;
const unlisteners: UnlistenFn[] = [];
const getRestoreRevision = getFrontendRestoreRevision;
let bookCacheActionGeneration = 0;
async function refreshShelfFromDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const restoreRevision = getRestoreRevision();
  const requestRevision = ++shelfReadRevision;
  const next = await readResource<ShelfResource>(descriptor);
  if (restoreRevision !== getRestoreRevision() || requestRevision !== shelfReadRevision) return;
  shelf.value = { ...next, groups: next.groups ?? [], books: next.books ?? [] };
}
async function refreshShelf(): Promise<void> {
  const restoreRevision = getRestoreRevision();
  const bootstrap = await appBootstrap();
  if (restoreRevision !== getRestoreRevision()) return;
  await refreshShelfFromDescriptor(bootstrap.shelf);
}
async function refreshSources(descriptor?: ResourceDescriptor): Promise<void> {
  const restoreRevision = getRestoreRevision();
  const requestRevision = ++sourcesReadRevision;
  if (descriptor) sourceDefinitionsResource = descriptor;
  if (!sourceDefinitionsResource) sourceDefinitionsResource = (await appBootstrap()).sources;
  const document = await readResource<SourceDefinitionsResource>(sourceDefinitionsResource);
  if (restoreRevision !== getRestoreRevision() || requestRevision !== sourcesReadRevision) return;
  sources.value = (document.sources ?? []).map(({ definitionJson: _definitionJson, ...source }) => source);
}
async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const restoreRevision = getRestoreRevision();
  const requestRevision = ++tasksReadRevision;
  const resource = descriptor ?? (await tasksResource()).resource;
  const document = await readResource<TasksResource>(resource);
  if (restoreRevision !== getRestoreRevision() || requestRevision !== tasksReadRevision) return;
  const byId = new Map((document.tasks ?? []).map((task) => [task.id, task]));
  for (const task of tasks.value) if (task.status === "recoveryRequired") byId.set(task.id, task);
  tasks.value = [...byId.values()];
}
async function loadPageResources(bootstrap?: AppBootstrap, expectedRevision = getRestoreRevision()): Promise<boolean> {
  const requestRevision = ++pageResourceRevision;
  const expectedShelfRevision = ++shelfReadRevision;
  const expectedSourcesRevision = ++sourcesReadRevision;
  const expectedTasksRevision = ++tasksReadRevision;
  const state = bootstrap ?? await appBootstrap();
  if (expectedRevision !== getRestoreRevision() || requestRevision !== pageResourceRevision
    || expectedShelfRevision !== shelfReadRevision || expectedSourcesRevision !== sourcesReadRevision
    || expectedTasksRevision !== tasksReadRevision) return false;
  sourceDefinitionsResource = state.sources;
  const tasksResourceResponse = await tasksResource();
  const [nextShelf, sourceDocument, taskDocument] = await Promise.all([
    readResource<ShelfResource>(state.shelf),
    readResource<SourceDefinitionsResource>(state.sources),
    readResource<TasksResource>(tasksResourceResponse.resource),
  ]);
  if (expectedRevision !== getRestoreRevision() || requestRevision !== pageResourceRevision
    || expectedShelfRevision !== shelfReadRevision || expectedSourcesRevision !== sourcesReadRevision
    || expectedTasksRevision !== tasksReadRevision) return false;
  shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
  sources.value = (sourceDocument.sources ?? []).map(({ definitionJson: _definitionJson, ...source }) => source);
  tasks.value = taskDocument.tasks ?? [];
  return true;
}
const canRefreshOpenedBookInfo = computed(() => {
  const book = openedBook.value;
  return book?.canChangeSource === true && Boolean(book.sourceId)
    && sources.value.some((source) => source.id === book.sourceId && source.enabled === true);
});
const bookDetailActions = useBookDetailActions({
  openedBook, readingBook, shelf, panelBusy: bookPanelBusy,
  infoRefreshBusy: bookInfoRefreshBusy, canRefreshInfo: canRefreshOpenedBookInfo,
  progressResetBusy: bookProgressResetBusy, displaySaveBusy: bookDisplaySaveBusy,
  singleRemovalBusy: singleBookRemovalBusy,
  recoveryRequired, recoveryTitle: shelfBatchRecoveryTitle, recoveryMessage: shelfBatchRecoveryMessage,
  pendingProgressReset: pendingBookProgressReset, progressResetError: bookProgressResetError,
  readerProgressDirty: readerSession.readerProgressDirty,
  refreshShelfFromDescriptor,
  saveCurrentProgress: readerSession.saveCurrentProgress,
  finishReadingSession: readerSession.finishReadingSession,
  closeReader: readerSession.closeReader,
  getRestoreRevision,
  notify, errorText,
});
const bookMetadataActions = useBookMetadataActions({
  openedBook, readingBook, shelf,
  editorBookId: bookDisplayEditorBookId, editorError: bookDisplayEditorError,
  saveBusy: bookDisplaySaveBusy, pendingRemoval, removalBusy: singleBookRemovalBusy,
  globalError: detailGlobalError, recoveryRequired,
  recoveryTitle: shelfBatchRecoveryTitle, recoveryMessage: shelfBatchRecoveryMessage,
  refreshShelfFromDescriptor,
  getRestoreRevision,
  notify, errorText,
});
const detailSession = useBookDetailSession({
  openedBook, shelf, bookPanelBusy,
  isRecoveryRequired: () => recoveryRequired.value,
  getRestoreRevision,
  startReading: readerSession.startReading,
  closeDisplayEditor: () => { bookDisplayEditorBookId.value = null; },
  refreshShelf,
  notify,
  errorText,
});
const {
  searchResultDetails,
  searchResultDetailsShelfBook,
  searchResultDetailsReadingBusy,
  searchResultBatchBusy,
} = detailSession;
const currentSearchResultDetails = computed(() => searchResultDetails.value ?? {
  result: openRequest?.kind === "search-result" ? openRequest.result : null,
  bookId: openRequest?.kind === "shelf-book" ? openRequest.bookId : null,
  initialTab: openRequest?.kind === "shelf-book" ? openRequest.initialTab ?? "catalog" : "info",
  error: pageResourceError.value,
});
let openRequestStarted = false;
async function startOpenRequestOnce(): Promise<void> {
  if (!openRequest || openRequestStarted) return;
  openRequestStarted = true;
  await detailSession.openRequest(openRequest);
}
function retainUnlistener(unlisten: UnlistenFn): void {
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}
async function setupPageEvents(): Promise<void> {
  try {
    retainUnlistener(await listen<ResourceDescriptor>("shelf-updated", async (event) => {
      try {
        await refreshShelfFromDescriptor(event.payload);
      } catch (error) {
        notify(`书架更新失败：${errorText(error)}`, "error");
      }
    }));
    retainUnlistener(await listen<ResourceDescriptor>("sources-updated", async (event) => {
      try {
        await refreshSources(event.payload);
      } catch (error) {
        notify(`书源刷新失败：${errorText(error)}`, "error");
      }
    }));
    retainUnlistener(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", async (event) => {
      if (event.payload.kind !== "tasks") return;
      try {
        await refreshTasks(event.payload.resource);
      } catch (error) {
        notify(`任务状态更新失败：${errorText(error)}`, "error");
      }
    }));
    retainUnlistener(await listen<{ taskId: string; resource: ResourceDescriptor }>("task-updated", async (event) => {
      const revision = getRestoreRevision();
      try {
        await refreshTasks(event.payload.resource);
        if (revision !== getRestoreRevision()) return;
        const task = tasks.value.find((entry) => entry.id === event.payload.taskId);
        if (!task || !task.bookId || !["completed", "failed", "recoveryRequired"].includes(task.status)) return;
        if (task.status === "recoveryRequired") {
          const message = task.result?.warning || task.error || "请重启应用后完成目录恢复。";
          lockShelfBatchRecovery("目录任务需要恢复", message);
          notify(message, "error");
          return;
        }
        if (openedBook.value?.id !== task.bookId) return;
        if (task.status === "failed") {
          notify(task.error || "目录任务失败。", "error");
          return;
        }
        if (task.kind === "refreshChapters" || task.kind === "checkNewChapters") {
          try {
            await bookMetadataActions.refreshOpenedBook(await getBook(task.bookId));
          } catch (error) {
            if (revision === getRestoreRevision()) notify(`目录已更新，但书籍信息刷新失败：${errorText(error)}`, "error");
          }
        }
      } catch (error) {
        if (revision === getRestoreRevision()) notify(`任务状态更新失败：${errorText(error)}`, "error");
      }
    }));
    retainUnlistener(await listen<Partial<AppTask>>("task-recovery-required", async (event) => {
      const task = event.payload;
      if (task.id && task.status === "recoveryRequired") {
        const existing = tasks.value.find((entry) => entry.id === task.id);
        if (existing) tasks.value = tasks.value.map((entry) => entry.id === task.id ? { ...entry, ...task } : entry);
        if (task.bookId === openedBook.value?.id) {
          const message = task.result?.warning || task.error || "请重启应用后完成目录恢复。";
          lockShelfBatchRecovery("目录任务需要恢复", message);
          notify(message, "error");
        }
      }
    }));
    retainUnlistener(await listen<{ bookId: string; book: ResourceDescriptor }>("chapters-prepared", async (event) => {
      const revision = getRestoreRevision();
      if (openedBook.value?.id !== event.payload.bookId) return;
      try {
        await bookMetadataActions.refreshOpenedBook(event.payload.book);
      } catch (error) {
        if (revision === getRestoreRevision()) notify(`读取更新后的目录失败：${errorText(error)}`, "error");
      }
    }));
    retainUnlistener(await listen<AppBootstrap>("app-state-updated", async (event) => {
      const revision = getRestoreRevision();
      const detailsWereOpen = Boolean(searchResultDetails.value);
      pageResourceLoading.value = true;
      pageResourceError.value = "";
      openedBook.value = null;
      bookCacheActionGeneration += 1;
      bookChapterCacheClearBusy.value = false;
      pendingBookChapterCacheClear.value = null;
      bookChapterCacheClearError.value = "";
      bookDisplayEditorBookId.value = null;
      pendingBookProgressReset.value = null;
      pendingRemoval.value = null;
      shelfReadRevision += 1;
      sourcesReadRevision += 1;
      tasksReadRevision += 1;
      try {
        const loaded = await loadPageResources(event.payload, revision);
        if (!loaded || revision !== getRestoreRevision()) return;
        if (detailsWereOpen) await detailSession.retry();
        else await startOpenRequestOnce();
      } catch (error) {
        if (revision !== getRestoreRevision()) return;
        pageResourceError.value = errorText(error);
        openedBook.value = null;
        notify(`恢复后的书架数据载入失败：${pageResourceError.value}`, "error");
      } finally {
        if (revision === getRestoreRevision()) {
          pageResourceLoading.value = false;
          if (!searchResultDetails.value) bookPanelBusy.value = false;
        }
      }
    }));
  } catch (error) {
    if (!disposed) notify(`详情页数据监听失败：${errorText(error)}`, "error");
  }
}
async function retryBookDetails(): Promise<void> {
  if (!pageResourceError.value) {
    await detailSession.retry();
    return;
  }
  const revision = getRestoreRevision();
  pageResourceLoading.value = true;
  bookPanelBusy.value = true;
  pageResourceError.value = "";
  try {
    if (await loadPageResources(undefined, revision)) await startOpenRequestOnce();
  } catch (error) {
    if (revision === getRestoreRevision()) pageResourceError.value = errorText(error);
  } finally {
    if (revision === getRestoreRevision()) {
      pageResourceLoading.value = false;
      if (!searchResultDetails.value) bookPanelBusy.value = false;
    }
  }
}
onMounted(async () => {
  pageResourceLoading.value = true;
  openedBook.value = null;
  try {
    await setupPageEvents();
    if (await loadPageResources()) {
      pageResourceError.value = "";
      openedBook.value = null;
      await startOpenRequestOnce();
    }
  } catch (error) {
    pageResourceError.value = errorText(error);
    bookPanelBusy.value = false;
  } finally {
    pageResourceLoading.value = false;
  }
  await nextTick();
  if (router.currentRoute.value.name === routeNames.bookDetail
    && !searchResultDetails.value && !openRequest) {
    void router.replace({ name: routeNames.shelf });
  }
});
onBeforeUnmount(() => {
  disposed = true;
  bookCacheActionGeneration += 1;
  pageResourceRevision += 1;
  shelfReadRevision += 1;
  sourcesReadRevision += 1;
  tasksReadRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
  detailSession.clear({ preserveOpenedBook: readerVisible.value });
  bookSourceSwitch.closeBookSourceSwitch();
});
function closeBookDetails(): void {
  detailSession.clear({ preserveOpenedBook: readerVisible.value });
  bookSourceSwitch.closeBookSourceSwitch();
  if (router.currentRoute.value.name !== routeNames.bookDetail) return;
  if (router.options.history.state.back) void router.back();
  else void router.replace({ name: routeNames.shelf });
}
watch(() => {
  const activeDetails = searchResultDetails.value;
  return Boolean(activeDetails?.inShelf && activeDetails.bookId
    && !shelf.value.books.some((book) => book.id === activeDetails.bookId));
}, (bookWasRemoved) => {
  if (bookWasRemoved) closeBookDetails();
});
const recentCatalogChapters = computed(() => (openedBook.value?.chapters ?? [])
  .slice()
  .sort((left, right) => right.index - left.index)
  .slice(0, 4));
const catalogRefreshBusy = computed(() => openedBook.value
  ? isBookTaskBusy(openedBook.value.id, "refreshChapters") : false);
const catalogCheckBusy = computed(() => openedBook.value
  ? isBookTaskBusy(openedBook.value.id, "checkNewChapters") : false);
const bookDetailTab = ref<"catalog" | "info" | "sources">("catalog");
const bookDetailMenuOpen = ref(false);
const catalogQuery = ref("");
const catalogDescending = ref(false);
const catalogSearchOpen = ref(false);
const bookDetailContentRef = ref<HTMLElement | null>(null);
const filteredCatalogChapters = computed(() => {
  const query = catalogQuery.value.trim().toLocaleLowerCase();
  const chapters = openedBook.value?.chapters ?? [];
  const filtered = query
    ? chapters.filter((chapter) => `${chapter.index + 1} ${chapter.title}`.toLocaleLowerCase().includes(query))
    : chapters;
  // Sorting is a view preference. Never mutate the Rust-owned chapter order.
  return filtered.slice().sort((left, right) => catalogDescending.value
    ? right.index - left.index : left.index - right.index);
});
function selectBookDetailTab(tab: "catalog" | "info" | "sources"): void {
  bookDetailTab.value = tab;
  bookDetailMenuOpen.value = false;
  void nextTick(() => {
    if (bookDetailContentRef.value) bookDetailContentRef.value.scrollTop = 0;
  });
}
async function locateCurrentCatalogChapter(): Promise<void> {
  const chapterIndex = openedBook.value?.progress?.chapterIndex;
  if (chapterIndex == null) return;
  catalogQuery.value = "";
  await nextTick();
  document.querySelector<HTMLButtonElement>(`[data-chapter-index="${chapterIndex}"]`)
    ?.scrollIntoView({ behavior: "smooth", block: "center" });
}
watch(() => searchResultDetails.value, (details) => {
  bookDetailTab.value = details?.initialTab ?? "catalog";
  bookDetailMenuOpen.value = false;
  catalogQuery.value = "";
  catalogSearchOpen.value = false;
}, { immediate: true });
watch(() => readerVisible.value, (visible) => {
  if (visible) bookDetailMenuOpen.value = false;
});
function isBookTaskBusy(bookId: string, kind: BookTaskKind): boolean {
  return tasks.value.some((task) => task.bookId === bookId && task.kind === kind
    && ["queued", "running", "pausing", "paused", "cancelling"].includes(task.status));
}
async function enqueueBookTask(bookId: string, action: BookTaskAction): Promise<void> {
  if (recoveryRequired.value) return;
  const revision = getRestoreRevision();
  try {
    const response = action === "download"
      ? await startBookDownload(bookId)
      : action === "refresh" ? await refreshChapters(bookId) : await checkNewChapters(bookId);
    if (revision !== getRestoreRevision()) return;
    await refreshTasks(response.resource);
    if (revision !== getRestoreRevision()) return;
    if (response.reused) notify("已有相同的目录任务，已复用当前任务。");
    else notify(action === "download" ? "整本缓存任务已加入。"
      : action === "refresh" ? "目录更新任务已加入。" : "新章节检查已加入。");
  } catch (error) {
    if (revision === getRestoreRevision()) notify(`无法创建任务：${errorText(error)}`, "error");
  }
}
async function enqueueCurrentBookTask(action: BookTaskAction): Promise<void> {
  const bookId = openedBook.value?.id;
  if (!bookId || recoveryRequired.value) return;
  await enqueueBookTask(bookId, action);
}
async function clearOpenedBookChapterCache(): Promise<void> {
  const pending = pendingBookChapterCacheClear.value;
  if (!pending || bookChapterCacheClearBusy.value || recoveryRequired.value) return;
  const revision = getRestoreRevision();
  const generation = ++bookCacheActionGeneration;
  const detailsAtStart = searchResultDetails.value;
  if (!detailsAtStart) return;
  const isCurrentOperation = () => revision === getRestoreRevision()
    && generation === bookCacheActionGeneration
    && detailsAtStart === searchResultDetails.value;
  const bookAtStart = openedBook.value;
  bookChapterCacheClearBusy.value = true;
  bookChapterCacheClearError.value = "";
  try {
    if (!bookAtStart || bookAtStart.id !== pending.id || bookAtStart.canChangeSource !== true || !bookAtStart.sourceId) {
      throw new Error(`《${pending.title}》不是可在线获取的书籍，不能清除章节缓存。`);
    }
    const readerBook = readingBook.value?.id === pending.id ? readingBook.value : null;
    if (readerBook) {
      await readerSession.saveCurrentProgress();
      if (!isCurrentOperation()) return;
      if (readingBook.value?.id !== pending.id || readerSession.readerProgressDirty.value) {
        throw new Error(`《${pending.title}》的阅读位置尚未保存，缓存未清除。`);
      }
      if (readerBook.progress) {
        openedBook.value = { ...bookAtStart, progress: { ...readerBook.progress } };
        shelf.value = {
          ...shelf.value,
          books: shelf.value.books.map((book) => book.id === pending.id
            ? { ...book, progress: { ...readerBook.progress! } } : book),
        };
      }
      await readerSession.finishReadingSession();
      if (!isCurrentOperation()) return;
      if (readingBook.value?.id === pending.id) await readerSession.closeReader(true);
      if (!isCurrentOperation()) return;
      if (readingBook.value?.id === pending.id || readerVisible.value) {
        throw new Error(`阅读器尚未关闭，未清除《${pending.title}》的章节缓存。`);
      }
    }

    const response = await clearBookChapterCacheApi(pending.id);
    if (!isCurrentOperation()) return;
    const warning = response.warning?.trim() ?? "";
    if (response.commitState === "notCommitted") {
      if (response.recoveryRequired) {
        pendingBookChapterCacheClear.value = null;
        lockShelfBatchRecovery("需要重启应用完成缓存恢复", `《${pending.title}》的章节缓存未清除，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`);
      } else {
        bookChapterCacheClearError.value = response.error || warning || "操作未提交，原缓存仍保留。";
      }
      throw new Error(bookChapterCacheClearError.value || shelfBatchRecoveryMessage.value);
    }
    if (response.commitState === "indeterminate") {
      pendingBookChapterCacheClear.value = null;
      lockShelfBatchRecovery("章节缓存清除状态未知", `《${pending.title}》的章节缓存清除提交状态未知。请重启应用检查书籍状态后再继续。${warning ? ` ${warning}` : ""}`);
      throw new Error(shelfBatchRecoveryMessage.value);
    }
    if (response.recoveryRequired || recoveryRequired.value) {
      pendingBookChapterCacheClear.value = null;
      if (openedBook.value?.id === pending.id) {
        openedBook.value = { ...openedBook.value, chapters: openedBook.value.chapters.map((chapter) => ({ ...chapter, resource: undefined })) };
      }
      if (response.recoveryRequired) lockShelfBatchRecovery("需要重启应用完成缓存恢复", `《${pending.title}》的章节缓存已清除，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`);
      throw new Error(shelfBatchRecoveryMessage.value || "缓存清理需要完成恢复。");
    }

    pendingBookChapterCacheClear.value = null;
    let updatedBook: BookResource | null = null;
    let readError = "";
    try {
      updatedBook = await readResource<BookResource>(response.book);
      if (!isCurrentOperation()) return;
      if (updatedBook.id !== pending.id) throw new Error("清除后返回的书籍与当前操作不匹配。");
    } catch (error) {
      if (!isCurrentOperation()) return;
      const firstError = errorText(error);
      try {
        updatedBook = await readResource<BookResource>(await getBook(pending.id));
        if (!isCurrentOperation()) return;
        if (updatedBook.id !== pending.id) throw new Error("重读后返回的书籍与当前操作不匹配。");
      } catch (reloadError) {
        if (!isCurrentOperation()) return;
        readError = `${firstError}；重读失败：${errorText(reloadError)}`;
      }
    }
    if (openedBook.value?.id === pending.id && openedBook.value !== bookAtStart && updatedBook) {
      try {
        updatedBook = await readResource<BookResource>(await getBook(pending.id));
        if (!isCurrentOperation()) return;
        if (updatedBook.id !== pending.id) throw new Error("重读后返回的书籍与当前操作不匹配。");
      } catch (error) {
        if (!isCurrentOperation()) return;
        readError = `并发更新后重读失败：${errorText(error)}`;
        updatedBook = null;
      }
    }
    if (openedBook.value?.id === pending.id) {
      openedBook.value = updatedBook ?? {
        ...openedBook.value,
        chapters: openedBook.value.chapters.map((chapter) => ({ ...chapter, resource: undefined })),
      };
    }
    if (readError) notify(`《${pending.title}》的章节缓存已清除，但详情未能重新载入：${readError}`, "error");
    else notify(response.clearedCount > 0
      ? `《${pending.title}》已清除 ${response.clearedCount} 章正文缓存，可以重新下载。`
      : `《${pending.title}》没有已缓存的章节正文。`);
  } catch (error) {
    if (isCurrentOperation()) {
      bookChapterCacheClearError.value = errorText(error);
      notify(bookChapterCacheClearError.value, "error");
    }
  } finally {
    if (generation === bookCacheActionGeneration) bookChapterCacheClearBusy.value = false;
  }
}
function openBookSourceSwitch(chapterTarget?: { id: string; title: string }): void {
  const book = openedBook.value;
  if (book) void bookSourceSwitch.openBookSourceSwitchForBook(book, chapterTarget);
}
const isMediaReaderBook = (book: BookResource | null | undefined): boolean =>
  book?.mediaType === "audio" || book?.mediaType === "video";
</script>
<template>
    <section class="book-detail-page">
      <article class="book-detail-panel">
        <header class="book-detail-toolbar">
          <BackButton class="book-detail-icon-button book-detail-back" label="返回" @click="closeBookDetails" />
          <nav class="book-detail-tabs">
            <button type="button" :class="{ active: bookDetailTab === 'info' }" @click="selectBookDetailTab('info')">概览</button>
            <button type="button" :class="{ active: bookDetailTab === 'catalog' }" @click="selectBookDetailTab('catalog')">{{ openedBook?.mediaType === 'video' ? '剧集' : openedBook?.mediaType === 'audio' ? '节目' : '目录' }}</button>
            <button type="button" :class="{ active: bookDetailTab === 'sources' }" @click="selectBookDetailTab('sources')">来源</button>
          </nav>
          <div v-if="openedBook" class="book-detail-toolbar-actions">
            <div class="book-detail-menu-wrap">
              <button type="button" class="book-detail-icon-button" @click.stop="bookDetailMenuOpen = !bookDetailMenuOpen"><PrototypeIcon name="more"/></button>
              <div v-if="bookDetailMenuOpen" class="book-detail-menu" @click.stop>
                <button v-if="!searchResultDetailsShelfBook && currentSearchResultDetails.result"
                  class="book-detail-menu-item"
                  :disabled="searchResultBatchBusy || recoveryRequired || detailSession.searchResultIsBusy(currentSearchResultDetails.result) || searchResultDetailsReadingBusy"
                  @click="bookDetailMenuOpen = false; detailSession.addSearchResultFromDetails()">
                  <PrototypeIcon name="plus"/><span>{{ detailSession.searchResultIsBusy(currentSearchResultDetails.result) ? '加入中…' : '加入内容库' }}</span>
                </button>
                <template v-if="searchResultDetailsShelfBook">
                <button class="book-detail-menu-item"
                  :disabled="bookDisplaySaveBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; bookMetadataActions.openBookDisplayEditor()">
                  <PrototypeIcon name="edit"/><span>编辑显示信息</span>
                </button>
                <button v-if="canRefreshOpenedBookInfo"
                  class="book-detail-menu-item"
                  :disabled="bookInfoRefreshBusy || bookPanelBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; bookDetailActions.refreshOpenedBookInfo()">
                  <PrototypeIcon name="refresh"/><span>{{ bookInfoRefreshBusy ? '刷新中…' : '刷新内容信息' }}</span>
                </button>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId" class="book-detail-menu-item"
                  :disabled="catalogCheckBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; enqueueCurrentBookTask('check')">
                  <PrototypeIcon name="search"/><span>{{ catalogCheckBusy ? '正在检查…' : '检查更新' }}</span>
                </button>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId" class="book-detail-menu-item"
                  :disabled="recoveryRequired"
                  @click="bookDetailMenuOpen = false; enqueueCurrentBookTask('download')">
                  <PrototypeIcon name="download"/><span>缓存全部{{ openedBook.mediaType === 'video' ? '剧集' : openedBook.mediaType === 'audio' ? '节目' : '章节' }}</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button class="book-detail-menu-item"
                  :disabled="bookProgressResetBusy || bookPanelBusy || bookDisplaySaveBusy || bookInfoRefreshBusy || singleBookRemovalBusy || recoveryRequired"
                  @click="pendingBookProgressReset = { id: openedBook.id, title: openedBook.title }; bookProgressResetError = ''; bookDetailMenuOpen = false">
                  <PrototypeIcon name="history"/><span>重置{{ openedBook.mediaType === 'video' ? '观看' : openedBook.mediaType === 'audio' ? '收听' : '阅读' }}进度</span>
                </button>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId"
                  class="book-detail-menu-item"
                  :disabled="bookChapterCacheClearBusy || bookPanelBusy || bookDisplaySaveBusy || bookInfoRefreshBusy || bookProgressResetBusy || singleBookRemovalBusy || bookSourceSwitch.bookSourceSwitchBusy || recoveryRequired"
                  @click="pendingBookChapterCacheClear = { id: openedBook.id, title: openedBook.title }; bookChapterCacheClearError = ''; bookDetailMenuOpen = false">
                  <PrototypeIcon name="trash"/><span>{{ bookChapterCacheClearBusy ? '清除中…' : '清除正文缓存' }}</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId" class="book-detail-menu-item"
                  :disabled="catalogRefreshBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; enqueueCurrentBookTask('refresh')">
                  <PrototypeIcon name="refresh"/><span>{{ catalogRefreshBusy ? '正在更新目录…' : '更新目录' }}</span>
                </button>
                <button v-if="openedBook.canChangeSource === true"
                  class="book-detail-menu-item" :disabled="bookPanelBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; openBookSourceSwitch()">
                  <PrototypeIcon name="source-switch"/><span>更换来源</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button class="book-detail-menu-item book-detail-menu-item--danger"
                  :disabled="singleBookRemovalBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; pendingRemoval = { id: openedBook.id, title: openedBook.title }">
                  <PrototypeIcon name="trash"/><span>从内容库移除</span>
                </button>
                </template>
              </div>
            </div>
          </div>
          <div v-else></div>
        </header>

        <main ref="bookDetailContentRef" class="book-detail-content"
          :class="{ 'book-detail-content--catalog': bookDetailTab === 'catalog' }"
          @click="bookDetailMenuOpen = false">
          <div v-if="bookPanelBusy || pageResourceLoading" class="catalog-empty">正在读取内容详情…</div>
          <div v-else-if="currentSearchResultDetails.error" class="catalog-empty">
            {{ currentSearchResultDetails.error }}
            <button v-if="currentSearchResultDetails.result" class="button secondary small" @click="retryBookDetails">重试</button>
            <button v-else-if="currentSearchResultDetails.bookId" class="button secondary small" @click="retryBookDetails">重试</button>
          </div>
          <template v-else-if="openedBook">
            <section v-if="bookDetailTab === 'catalog'" class="book-catalog-view">
              <div class="prototype-catalog-tools">
                <label class="catalog-search">
                  <PrototypeIcon name="search"/>
                  <input v-model="catalogQuery"
                    :placeholder="openedBook.mediaType === 'video' ? '搜索剧集' : openedBook.mediaType === 'audio' ? '搜索节目' : '搜索章节'"/>
                  <button v-if="catalogQuery" type="button"
                    @click="catalogQuery = ''"><PrototypeIcon name="close"/></button>
                </label>
                <div class="catalog-tools-actions">
                  <button type="button" class="button secondary small"
                    :disabled="!hasReadingProgress(openedBook.progress)"
                    @click="locateCurrentCatalogChapter"><PrototypeIcon name="tune"/> <span>定位当前</span></button>
                  <button type="button" class="button secondary small"
                    @click="catalogDescending = !catalogDescending">
                    <PrototypeIcon name="filter"/><span>排序</span>
                  </button>
                  <button type="button" class="button secondary small"
                    :disabled="!searchResultDetailsShelfBook || openedBook.canChangeSource !== true || !openedBook.sourceId || recoveryRequired"
                    :title="openedBook.mediaType === 'video' ? '缓存全部剧集' : openedBook.mediaType === 'audio' ? '缓存全部节目' : '缓存全部章节'"
                    @click="enqueueCurrentBookTask('download')"><PrototypeIcon name="download"/><span>缓存全部</span></button>
                </div>
              </div>
              <div class="catalog-list">
                <article v-for="chapter in filteredCatalogChapters" :key="chapter.id"
                  class="catalog-row-wrap"
                  :class="{ current: hasReadingProgress(openedBook.progress) && chapter.index === openedBook.progress?.chapterIndex }">
                  <button :data-chapter-index="chapter.index" type="button"
                    class="catalog-row"
                    :disabled="searchResultDetailsReadingBusy || recoveryRequired"
                    @click="detailSession.startReading(chapter.index)">
                    <span class="catalog-chapter-main">
                      <strong>{{ chapter.title }}</strong>
                    </span>
                    <span v-if="hasReadingProgress(openedBook.progress) && chapter.index === openedBook.progress?.chapterIndex"
                      class="catalog-current-label">当前</span>
                    <span v-if="!chapter.resource" class="catalog-uncached"
                      :title="openedBook.canChangeSource === true ? '正文尚未准备，打开时由内容引擎读取' : '内容尚未准备'"
><PrototypeIcon name="cloud"/></span>
                    <span v-else class="catalog-cache-space"></span>
                  </button>
                  <button v-if="searchResultDetailsShelfBook && openedBook.canChangeSource === true && !isMediaReaderBook(openedBook)"
                    type="button" class="catalog-replace-action"
title="更换章节来源"
                    :disabled="recoveryRequired"
                    @click="openBookSourceSwitch({ id: chapter.id, title: chapter.title })">
                    <PrototypeIcon name="source-switch"/>
                  </button>
                </article>
                <div v-if="!openedBook.chapters?.length" class="catalog-empty">暂无可显示的目录。</div>
                <div v-else-if="!filteredCatalogChapters.length" class="catalog-empty">
                  没有匹配的内容。<button type="button" class="text-button"
                    @click="catalogQuery = ''; catalogSearchOpen = false">清空搜索</button>
                </div>
              </div>
            </section>

            <section v-else-if="bookDetailTab === 'sources'" class="detail-sources-page"
>
              <section class="detail-source-block">
                <h2>当前来源</h2>
                <article class="detail-source-current">
                  <span class="detail-source-glyph"><PrototypeIcon name="source"/></span>
                  <div>
                    <strong>{{ openedBook.sourceName || '本地内容' }}</strong>
                    <small v-if="openedBook.sourceGroup">{{ openedBook.sourceGroup }}</small>
                    <small v-if="openedBook.sourceId && sources.some(source => source.id === openedBook?.sourceId)">
                      {{ sources.find(source => source.id === openedBook?.sourceId)?.enabled ? '已启用' : '已停用' }} · 本机已导入
                    </small>
                    <small v-else-if="openedBook.sourceId">当前来源尚未在本机导入或已被移除</small>
                    <small v-else>本地文件，不需要网络来源</small>
                  </div>
                  <button v-if="canRefreshOpenedBookInfo" type="button" class="button secondary small"
                    :disabled="bookInfoRefreshBusy || recoveryRequired"
                    @click="bookDetailActions.refreshOpenedBookInfo()">
                    {{ bookInfoRefreshBusy ? '刷新中…' : '刷新信息' }}
                  </button>
                </article>
              </section>
              <section v-if="openedBook.canChangeSource === true" class="detail-source-block detail-alternates">
                <div class="detail-alternates-heading">
                  <h2>可替换来源</h2>
                  <button type="button"
                    :disabled="bookSourceSwitch.bookSourceSwitchBusy || recoveryRequired"
                    @click="openBookSourceSwitch()">
                    <PrototypeIcon name="search"/><span>{{ bookSourceSwitch.bookSourceSwitchBook?.id === openedBook.id && bookSourceSwitch.bookSourceCandidateResults.length ? '重新搜索' : '搜索候选' }}</span>
                  </button>
                </div>
                <div v-if="bookSourceSwitch.bookSourceSwitchBook?.id === openedBook.id && bookSourceSwitch.bookSourceCandidateResults.length"
                  class="detail-alternates-list">
                  <article v-for="candidate in bookSourceSwitch.bookSourceCandidateResults" :key="candidate.resultId"
                    class="detail-alternate-row">
                    <div>
                      <strong>{{ candidate.sourceName }}</strong>
                      <span>{{ candidate.title }}{{ candidate.author ? ' · ' + candidate.author : ' · 作者未提供' }}</span>
                      <small v-if="candidate.requiresIdentityConfirmation">作者未验证，更换前必须确认身份</small>
                      <small v-else-if="candidate.latestChapter">最新：{{ candidate.latestChapter }}</small>
                    </div>
                    <button v-if="candidate.sourceId !== openedBook.sourceId" type="button"
                      :disabled="bookSourceSwitch.bookSourceSwitchBusy || recoveryRequired"
                      @click="bookSourceSwitch.selectBookSourceCandidate(candidate); bookSourceSwitch.bookSourceSwitchOpen = true">
                      切换
                    </button>
                    <span v-else class="detail-source-current-badge">当前</span>
                  </article>
                </div>
                <div v-else class="detail-alternates-empty">
                  尚未搜索到可用候选。点击「搜索候选」使用已启用书源进行真实匹配；结果不会包含推测的响应时间。
                </div>
                <p v-if="bookSourceSwitch.bookSourceSwitchBook?.id === openedBook.id && bookSourceSwitch.bookSourceCandidateErrors.length"
                  class="detail-source-partial">
                  部分书源搜索失败（{{ bookSourceSwitch.bookSourceCandidateErrors.length }} 个），可在候选搜索页面查看错误。
                </p>
                <button v-if="searchResultDetailsShelfBook" type="button"
                  class="detail-source-check" :disabled="catalogCheckBusy || recoveryRequired"
                  @click="enqueueCurrentBookTask('check')">
                  {{ catalogCheckBusy ? '检查更新中…' : '检查内容更新' }}
                </button>
              </section>
            </section>

            <section v-else class="book-info-view">
              <div class="detail-hero">
                <img v-if="openedBook.coverSrc || currentSearchResultDetails.result?.coverSrc"
                  :src="openedBook.coverSrc || currentSearchResultDetails.result?.coverSrc"
loading="lazy" decoding="async"/>
                <div v-else class="detail-cover-fallback">{{ openedBook.title.slice(0, 1) }}</div>
                <div class="detail-title-copy">
                  <small class="detail-type-label">
                    {{ openedBook.mediaType === 'video' ? '视频'
                      : openedBook.mediaType === 'audio' ? '音频'
                      : openedBook.kind === 'comic' ? '漫画' : '小说' }}
                    {{ openedBook.kind ? ' · ' + openedBook.kind : '' }}
                  </small>
                  <h2>{{ openedBook.title }}</h2>
                  <p class="detail-author">{{ openedBook.author || '作者未知' }}</p>
                  <div class="detail-hero-actions">
                    <button class="button primary" type="button"
                      :disabled="bookPanelBusy || searchResultDetailsReadingBusy || recoveryRequired || !(openedBook.chapterCount || openedBook.chapters?.length)"
                      @click="detailSession.startReading()">
                      <PrototypeIcon :name="openedBook.mediaType === 'video' ? 'play' : openedBook.mediaType === 'audio' ? 'speaker' : 'book'"/>
                      {{ openedBook.mediaType === 'video'
                        ? hasReadingProgress(openedBook.progress) ? '继续观看' : '开始观看'
                        : openedBook.mediaType === 'audio'
                        ? hasReadingProgress(openedBook.progress) ? '继续收听' : '开始收听'
                        : hasReadingProgress(openedBook.progress) ? '继续阅读' : '开始阅读' }}
                    </button>
                    <button v-if="!searchResultDetailsShelfBook && currentSearchResultDetails.result"
                      type="button" class="button secondary"
                      :disabled="searchResultBatchBusy || recoveryRequired || detailSession.searchResultIsBusy(currentSearchResultDetails.result) || searchResultDetailsReadingBusy"
                      @click="detailSession.addSearchResultFromDetails()">
                      {{ detailSession.searchResultIsBusy(currentSearchResultDetails.result) ? '加入中…' : '加入内容库' }}
                    </button>
                  </div>
                </div>
                <aside class="prototype-overview-progress">
                  <div>
                    <small>{{ openedBook.mediaType === 'video' ? '观看进度' : openedBook.mediaType === 'audio' ? '收听进度' : '阅读进度' }}</small>
                    <strong v-if="hasReadingProgress(openedBook.progress)">
                      第 {{ (openedBook.progress?.chapterIndex ?? 0) + 1 }}
                      {{ openedBook.mediaType === 'video' ? '集' : openedBook.mediaType === 'audio' ? '期' : openedBook.kind === 'comic' ? '话' : '章' }}
                      · {{ readingProgressPercent(openedBook.progress, openedBook.chapterCount || openedBook.chapters.length) }}%
                    </strong>
                    <strong v-else>尚未开始</strong>
                  </div>
                  <div class="detail-reading-track"

>
                    <span :style="{ width: `${readingProgressPercent(openedBook.progress, openedBook.chapterCount || openedBook.chapters.length)}%` }"></span>
                  </div>
                  <button type="button" class="detail-catalog-link" @click="selectBookDetailTab('catalog')">
                    查看{{ openedBook.mediaType === 'video' ? '剧集' : openedBook.mediaType === 'audio' ? '节目' : '目录' }}
                    <PrototypeIcon name="right"/>
                  </button>
                </aside>
              </div>
              <div class="book-detail-facts">
                <button type="button" class="book-detail-fact detail-fact-link"
                  @click="selectBookDetailTab('catalog')">
                  <span><small>最新</small><strong>{{ openedBook.latestChapter || recentCatalogChapters[0]?.title || '暂无目录信息' }}</strong></span>
                  <PrototypeIcon name="right"/>
                </button>
                <div class="book-detail-fact">
                  <span><small>规模</small><strong>{{ openedBook.wordCount ? openedBook.wordCount : `${openedBook.chapterCount || openedBook.chapters.length} 项内容` }}</strong></span>
                </div>
                <button type="button" class="book-detail-fact detail-fact-link"
                  @click="selectBookDetailTab('sources')">
                  <span><small>当前来源</small><strong>{{ openedBook.sourceName || '本地内容' }}</strong></span>
                  <PrototypeIcon name="right"/>
                </button>
                <div class="book-detail-fact">
                  <span><small>类型</small><strong>{{ openedBook.kind || (openedBook.mediaType === 'video' ? '视频' : openedBook.mediaType === 'audio' ? '音频' : '阅读内容') }}</strong></span>
                </div>
              </div>
              <section class="detail-section detail-intro">
                <header><h3>简介</h3></header>
                <p>{{ openedBook.intro || '暂无简介。' }}</p>
              </section>
            </section>
          </template>
          <div v-else class="catalog-empty">暂无可显示的内容信息。</div>
        </main>



        <BookDisplayMetadataEditor
          v-if="openedBook && bookDisplayEditorBookId === openedBook.id"
          :key="openedBook.id"
          :book="openedBook"
          :busy="bookDisplaySaveBusy"
          :locked="recoveryRequired"
          :error="bookDisplayEditorError"
          :return-label="bookDetailTab === 'catalog' ? '返回目录' : '返回概览'"
          @close="bookMetadataActions.closeBookDisplayEditor"
          @save="bookMetadataActions.saveBookDisplayMetadata"
        />
      </article>
    </section>

    <section
      v-if="pendingBookProgressReset"
      class="modal-backdrop"
      @click.self="!bookProgressResetBusy && (pendingBookProgressReset = null)"
    >
      <article class="app-modal confirm-modal">
        <div class="confirm-symbol">↺</div>
        <h2 id="book-reset-progress-title">重置阅读进度？</h2>
        <p>将清除《{{ pendingBookProgressReset.title }}》的阅读位置，从第一章重新开始。若当前阅读器打开了这本书，会先保存当前位置再关闭阅读器。</p>
        <p>章节目录、已缓存正文、书源、书签和阅读历史都会保留。</p>
        <p v-if="bookProgressResetError" class="source-identity-confirmation">{{ bookProgressResetError }}</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="bookProgressResetBusy" @click="pendingBookProgressReset = null">取消</button>
          <button class="button danger" :disabled="bookProgressResetBusy || recoveryRequired" @click="bookDetailActions.resetOpenedBookProgress">
            {{ bookProgressResetBusy ? '正在重置…' : '确认重置进度' }}
          </button>
        </div>
      </article>
    </section>

    <section
      v-if="pendingBookChapterCacheClear"
      class="modal-backdrop"
      @click.self="!bookChapterCacheClearBusy && (pendingBookChapterCacheClear = null)"
    >
      <article class="app-modal confirm-modal">
        <div class="confirm-symbol">⌫</div>
        <h2 id="book-clear-chapter-cache-title">清除章节缓存？</h2>
        <p>将清除《{{ pendingBookChapterCacheClear.title }}》已缓存的章节正文。之后可以重新下载正文。</p>
        <p>阅读进度、章节目录、封面和书签都会保留。<span v-if="readingBook?.id === pendingBookChapterCacheClear.id">当前阅读器会先保存阅读位置并关闭，避免继续显示已清除的正文。</span></p>
        <p v-if="bookChapterCacheClearError" class="source-identity-confirmation">{{ bookChapterCacheClearError }}</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="bookChapterCacheClearBusy" @click="pendingBookChapterCacheClear = null">取消</button>
          <button class="button danger" :disabled="bookChapterCacheClearBusy || recoveryRequired" @click="clearOpenedBookChapterCache">
            {{ bookChapterCacheClearBusy ? '正在清除…' : '确认清除缓存' }}
          </button>
        </div>
      </article>
    </section>

    <section
      v-if="pendingRemoval"
      class="modal-backdrop"
      @click.self="!singleBookRemovalBusy && (pendingRemoval = null)"
    >
      <article class="app-modal confirm-modal">
        <div class="confirm-symbol">⌫</div>
        <h2>从书架移除这本书？</h2>
        <p>《{{ pendingRemoval.title }}》及其本地书籍数据、阅读进度和书签都会被移除。</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="singleBookRemovalBusy" @click="pendingRemoval = null">保留</button>
          <button class="button danger" :disabled="singleBookRemovalBusy || recoveryRequired" @click="bookMetadataActions.removeBookFromShelf">
            {{ singleBookRemovalBusy ? '正在移除…' : '确认移除' }}
          </button>
        </div>
      </article>
    </section>

    <BookSourceSwitchSheet
      :book-source-switch="bookSourceSwitch"
      :book-source-candidates="bookSourceCandidates"
      :source-name="sourceName"
      :shelf-batch-recovery-required="recoveryRequired"
    />

</template>

<style scoped>
.book-detail-page {
  display: flex;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  justify-content: center;
  overflow: hidden;
  padding-top: var(--safe-top);
  background: var(--app-background);
}

.book-detail-panel {
  display: flex;
  width: min(920px, 100%);
  height: 100%;
  min-height: 0;
  flex-direction: column;
  overflow: hidden;
  border: 0;
  border-radius: 0;
  background: var(--app-background);
}

.book-detail-toolbar {
  position: relative;
  z-index: 2;
  display: grid;
  min-height: 64px;
  grid-template-columns: 42px minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  padding: 0 16px;
  border-bottom: 1px solid var(--app-line);
  background: var(--app-background);
}

.book-detail-tabs {
  display: flex;
  width: max-content;
  max-width: 100%;
  height: 100%;
  justify-self: center;
  gap: 14px;
}

.book-detail-tabs button {
  position: relative;
  min-width: 56px;
  height: 100%;
  padding: 0 8px;
  color: var(--app-muted);
  font-size: 10px;
  cursor: pointer;
}

.book-detail-tabs button.active {
  color: var(--app-text);
  font-weight: 720;
}

.book-detail-tabs button.active::after {
  position: absolute;
  right: 10px;
  bottom: 0;
  left: 10px;
  height: 2px;
  border-radius: 2px;
  background: var(--app-accent);
  content: "";
}

.book-detail-toolbar-actions {
  display: flex;
  min-width: 36px;
  align-items: center;
  justify-content: flex-end;
}

.book-detail-menu-wrap {
  position: relative;
}

.book-detail-icon-button {
  display: grid;
  width: 36px;
  height: 36px;
  place-items: center;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  cursor: pointer;
}

.book-detail-icon-button:hover {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.book-detail-menu {
  position: absolute;
  z-index: 5;
  top: calc(100% + 8px);
  right: 0;
  display: grid;
  width: min(260px, calc(100vw - 36px));
  gap: 2px;
  padding: 7px;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
}

.book-detail-menu-item {
  display: flex;
  min-height: 40px;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border-radius: 8px;
  color: var(--app-text);
  text-align: left;
  cursor: pointer;
}

.book-detail-menu-item:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.book-detail-menu-item--danger {
  color: var(--app-danger);
}

.book-detail-menu-divider {
  height: 1px;
  margin: 4px 3px;
  background: var(--app-line);
}

.book-detail-content {
  min-height: 0;
  flex: 1;
  overflow: auto;
  padding: 24px;
  color: var(--app-text);
  background: var(--app-background);
  overscroll-behavior: contain;
}

.detail-sources-page {
  display: grid;
  gap: 18px;
}

.book-catalog-view {
  display: block;
}

.book-info-view {
  display: block;
}

.detail-hero {
  display: grid;
  grid-template-columns: 160px minmax(0, 1fr) 190px;
  align-items: start;
  gap: 22px;
}

.detail-hero > img,
.detail-cover-fallback {
  display: block;
  width: 100%;
  aspect-ratio: 4 / 5;
  overflow: hidden;
  border-radius: 12px;
  object-fit: cover;
}

.detail-cover-fallback {
  display: grid;
  place-items: center;
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
  font-size: 40px;
  font-weight: 700;
}

.detail-title-copy {
  display: grid;
  min-width: 0;
  align-content: start;
  gap: 0;
  padding-top: 5px;
}

.detail-type-label,
.detail-author,
.prototype-overview-progress small,
.book-detail-fact small {
  color: var(--app-muted);
  font-size: 12px;
}

.detail-type-label {
  color: var(--app-accent);
  font-size: 8px;
  font-weight: 750;
}

.detail-title-copy h2 {
  margin: 7px 0 5px;
  font-size: 29px;
  line-height: 1.15;
  letter-spacing: -0.04em;
  overflow-wrap: anywhere;
}

.detail-author {
  margin: 0;
  font-size: 10px;
}

.detail-hero-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 22px;
}

.prototype-overview-progress {
  display: flex;
  min-height: 126px;
  flex-direction: column;
  padding: 15px;
  border-left: 1px solid var(--app-line);
}

.prototype-overview-progress > div:first-child,
.book-detail-fact > span {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 5px;
}

.prototype-overview-progress small,
.book-detail-fact small {
  font-size: 8px;
}

.prototype-overview-progress strong {
  font-size: 11px;
}

.detail-reading-track {
  height: 5px;
  margin-top: auto;
  overflow: hidden;
  border-radius: 999px;
  background: var(--app-panel-soft);
}

.detail-reading-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--app-accent);
}

.detail-catalog-link {
  display: flex;
  min-height: 30px;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  color: var(--app-accent-ink);
  font-size: 9px;
  cursor: pointer;
}

.book-detail-facts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin-top: 28px;
  border-top: 1px solid var(--app-line);
  border-bottom: 1px solid var(--app-line);
}

.book-detail-fact {
  display: grid;
  min-width: 0;
  min-height: 64px;
  grid-template-columns: minmax(0, 1fr) 16px;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  border: 0;
  border-right: 1px solid var(--app-line);
  border-radius: 0;
  color: var(--app-text);
  background: transparent;
  text-align: left;
}

.book-detail-fact:nth-child(2n) {
  border-right: 0;
}

.book-detail-fact:nth-child(-n + 2) {
  border-bottom: 1px solid var(--app-line);
}

.book-detail-fact:not(.detail-fact-link) {
  grid-template-columns: minmax(0, 1fr);
}

.detail-fact-link {
  cursor: pointer;
}

.detail-fact-link:hover {
  border-color: var(--app-accent);
  background: var(--app-accent-soft);
}

.book-detail-fact strong {
  overflow: hidden;
  font-size: 9px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.detail-source-block {
  display: grid;
  gap: 10px;
  padding: 16px;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  background: var(--app-surface);
}

.detail-section {
  display: grid;
  gap: 8px;
  margin-top: 28px;
}

.detail-section header,
.detail-alternates-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.detail-section h3,
.detail-source-block h2 {
  margin: 0;
  font-size: 11px;
}

.detail-section p {
  margin: 0;
  color: var(--app-muted);
  font-size: 10px;
  line-height: 1.85;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.prototype-catalog-tools {
  position: sticky;
  top: 0;
  z-index: 1;
  display: grid;
  grid-template-columns: minmax(240px, 300px) minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
  padding: 10px 0;
  border-bottom: 1px solid var(--app-line);
  background: var(--app-background);
}

.catalog-search {
  display: flex;
  min-height: 36px;
  align-items: center;
  gap: 7px;
  padding: 0 9px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
  color: var(--app-muted);
  background: var(--app-surface);
}

.catalog-search input {
  width: 100%;
  min-width: 0;
  border: 0;
  outline: 0;
  color: var(--app-text);
  background: transparent;
  font-size: 9px;
}

.catalog-search button {
  display: grid;
  width: 30px;
  height: 30px;
  flex: none;
  place-items: center;
  border-radius: 8px;
  color: var(--app-muted);
  cursor: pointer;
}

.catalog-search button:hover {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.catalog-tools-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.catalog-tools-actions .button {
  min-height: 34px;
  flex: 1;
  padding: 0 6px;
  border: 0;
  border-radius: 8px;
  color: var(--app-muted);
  background: var(--app-panel-soft);
  font-size: 9px;
}

.catalog-list {
  margin: 0 13px;
}

.catalog-row-wrap {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 38px;
  gap: 4px;
  border-bottom: 1px solid var(--app-line);
}

.catalog-row {
  display: grid;
  min-width: 0;
  min-height: 52px;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 8px;
  padding: 0 4px;
  border: 0;
  border-radius: 0;
  color: var(--app-text);
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.catalog-row:hover:not(:disabled),
.catalog-row-wrap.current .catalog-row {
  background: var(--app-accent-soft);
}

.catalog-row-wrap.current::before {
  position: absolute;
  top: 12px;
  bottom: 12px;
  left: 0;
  width: 2px;
  border-radius: 2px;
  background: var(--app-accent);
  content: "";
}

.catalog-chapter-main {
  min-width: 0;
}

.catalog-chapter-main strong {
  display: block;
  overflow: hidden;
  font-size: 10px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.catalog-current-label {
  color: var(--app-accent-ink);
  font-size: 9px;
  white-space: nowrap;
}

.catalog-uncached {
  display: grid;
  width: 20px;
  height: 20px;
  place-items: center;
  color: var(--app-muted);
}

.catalog-uncached .icon {
  width: 15px;
  height: 15px;
}

.catalog-replace-action {
  display: grid;
  width: 30px;
  height: 30px;
  place-items: center;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  cursor: pointer;
}

.catalog-replace-action:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.catalog-empty {
  display: grid;
  min-height: 150px;
  place-content: center;
  justify-items: center;
  gap: 12px;
  color: var(--app-muted);
  text-align: center;
}

.detail-sources-page {
  align-content: start;
  gap: 0;
  max-width: 850px;
  margin: 0 auto;
}

.detail-source-current,
.detail-alternate-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
}

.detail-source-current > div,
.detail-alternate-row > div {
  display: grid;
  min-width: 0;
  gap: 2px;
}

.detail-source-current small,
.detail-alternate-row span,
.detail-alternate-row small,
.detail-alternates-empty,
.detail-source-partial {
  color: var(--app-muted);
  font-size: 8px;
}

.detail-source-block {
  gap: 10px;
  margin-top: 22px;
  padding: 18px 0 0;
  border: 0;
  border-top: 1px solid var(--app-line);
  border-radius: 0;
  background: transparent;
}

.detail-source-block:first-child {
  margin-top: 0;
  padding-top: 0;
  border-top: 0;
}

.detail-source-current {
  min-height: 82px;
  grid-template-columns: 44px minmax(0, 1fr) auto;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  background: var(--app-surface);
}

.detail-source-current strong {
  font-size: 11px;
}

.detail-source-glyph {
  display: grid;
  width: 40px;
  height: 40px;
  place-items: center;
  border-radius: 13px;
  color: var(--app-accent);
  background: var(--app-accent-soft);
}

.detail-alternates-heading > button,
.detail-alternate-row > button,
.detail-source-check {
  display: inline-flex;
  min-height: 34px;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 0 10px;
  border: 1px solid var(--app-line);
  border-radius: 9px;
  color: var(--app-accent-ink);
  background: var(--app-surface);
  cursor: pointer;
}

.detail-alternates-list {
  display: grid;
  border-top: 1px solid var(--app-line);
}

.detail-alternate-row {
  min-width: 0;
  min-height: 58px;
  padding: 8px 0;
  border-bottom: 1px solid var(--app-line);
}

.detail-alternate-row strong {
  font-size: 9px;
}

.detail-alternate-row > button {
  min-height: 28px;
  padding: 0 10px;
  border: 0;
  border-radius: 8px;
  background: var(--app-panel-soft);
  font-size: 8px;
}

.detail-source-current-badge {
  color: var(--app-success);
}

.detail-alternates-empty {
  padding: 12px 0;
  line-height: 1.5;
}

.detail-source-partial {
  margin: 0;
  color: var(--app-danger);
}

.catalog-empty .button {
  margin-top: 4px;
}
@media (max-width: 860px) {
  .book-detail-panel {
    width: 100%;
    border: 0;
    border-radius: 0;
  }

  .book-detail-toolbar {
    min-height: 58px;
    gap: 10px;
    padding: 0 10px;
  }

  .book-detail-content {
    padding: 22px 13px calc(66px + var(--safe-bottom));
  }

  .book-detail-content--catalog {
    padding: 0 13px calc(66px + var(--safe-bottom));
  }

  .book-info-view {
    display: block;
  }

  .detail-hero {
    grid-template-columns: 100px minmax(0, 1fr);
    gap: 14px;
  }

  .detail-title-copy {
    padding-top: 1px;
  }

  .detail-title-copy h2 {
    margin-top: 6px;
    font-size: 21px;
  }

  .prototype-overview-progress {
    grid-column: 1 / -1;
    min-height: 0;
    padding: 12px 0 0;
    border-top: 1px solid var(--app-line);
    border-left: 0;
  }

  .detail-reading-track {
    margin-top: 10px;
  }

  .detail-catalog-link {
    margin-top: 5px;
  }

  .book-detail-facts {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    margin-top: 20px;
  }

  .book-detail-fact {
    min-height: 58px;
    padding: 0 9px;
  }

  .detail-section {
    margin-top: 22px;
  }

  .detail-hero-actions .button {
    min-height: 36px;
    padding: 0 10px;
    font-size: 10px;
  }

  .prototype-catalog-tools {
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
    margin: 0 0 12px;
    padding: 10px 12px 9px;
    box-shadow: 0 5px 12px rgb(32 34 42 / 4%);
  }

  .catalog-tools-actions {
    justify-content: flex-start;
  }

  .catalog-row-wrap {
    grid-template-columns: minmax(0, 1fr) 34px;
  }

  .catalog-row {
    min-height: 56px;
    padding: 0 2px;
  }

  .book-detail-menu {
    position: fixed;
    right: 10px;
    bottom: calc(10px + var(--safe-bottom));
    left: 10px;
    top: auto;
    width: auto;
    padding: 8px;
    border-radius: 16px;
  }

  .book-detail-menu-item {
    min-height: 42px;
  }
}

@media (max-width: 430px) {
  .book-detail-toolbar {
    grid-template-columns: 38px minmax(0, 1fr) 34px;
    gap: 6px;
    padding-right: 8px;
    padding-left: 8px;
  }

  .book-detail-toolbar .back-button {
    width: 38px;
    min-width: 38px;
    height: 38px;
    min-height: 38px;
  }

  .book-detail-toolbar-actions {
    min-width: 34px;
  }

  .book-detail-tabs {
    gap: 3px;
  }

  .book-detail-tabs button {
    min-width: 50px;
    padding: 0 4px;
    font-size: 9px;
  }

  .catalog-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .catalog-row .catalog-cache-space {
    display: none;
  }
}

</style>
