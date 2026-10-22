<script setup lang="ts">
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sources } from "../sources/sourcesStore";
import { clearBookChapterCache as clearBookChapterCacheApi } from "../../api/books";
import { moreInformation } from "../../api/moreInformation";
import { mainText } from "../../api/mainText";
import type { AppBootstrap, BookResource, SearchBookResult, SearchResultSource } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import router, { routeNames } from "../../router";
import BackButton from "../../ui/BackButton.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BookDisplayMetadataEditor from "./BookDisplayMetadataEditor.vue";
import { useBookDetailActions } from "./useBookDetailActions";
import { useBookMetadataActions } from "./useBookMetadataActions";
import { hasReadingProgress, readingProgressPercent } from "./readingProgress";
import { getReaderSession } from "../reader/readerSession";
import { getBookSourceSwitchSession } from "../sources/bookSourceSwitchSession";
import BookSourceSwitchView from "../sources/BookSourceSwitchView.vue";
import BookSourcesPanel from "../sources/BookSourcesPanel.vue";
import BookCatalogPanel from "./BookCatalogPanel.vue";
import { consumeBookDetailOpenRequest, returnFromBookDetail } from "./bookDetailOpenRequest";
import { useBookDetailSession } from "./useBookDetailSession";
import { clearOpenedBook, openedBook, replaceOpenedBook, refreshOpenedBook } from "./bookDetailState";
import { errorText, notify } from "../../app/notifications";
import { getFrontendRestoreRevision, lockShelfBatchRecovery, shelfBatchRecoveryMessage, shelfBatchRecoveryRequired } from "../../app/recoveryState";

type BookAction = "download" | "refresh" | "check";
function progressTime(milliseconds = 0): string {
  const seconds = Math.floor(Math.max(0, milliseconds) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
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
const pendingRemoval = ref<{ id: string; title: string } | null>(null);
const bookChapterCacheClearBusy = ref(false);
const bookChapterCacheClearError = ref("");
const pendingBookChapterCacheClear = ref<{ id: string; title: string } | null>(null);
let disposed = false;
let closing = false;
const unlisteners: UnlistenFn[] = [];
const getRestoreRevision = getFrontendRestoreRevision;
let bookCacheActionGeneration = 0;
const canRefreshOpenedBookInfo = computed(() => {
  const book = openedBook.value;
  return book?.canChangeSource === true && Boolean(book.sourceId)
    && sources.value.some((source) => source.id === book.sourceId && source.enabled === true);
});
const bookDetailActions = useBookDetailActions({
  readingBook, panelBusy: bookPanelBusy,
  infoRefreshBusy: bookInfoRefreshBusy, canRefreshInfo: canRefreshOpenedBookInfo,
  progressResetBusy: bookProgressResetBusy,
  recoveryRequired,
  pendingProgressReset: pendingBookProgressReset, progressResetError: bookProgressResetError,
  readerProgressDirty: readerSession.readerProgressDirty,
  saveCurrentProgress: readerSession.saveCurrentProgress,
  finishReadingSession: readerSession.finishReadingSession,
  closeReader: readerSession.closeReader,
  getRestoreRevision,
  notify, errorText,
});
const bookMetadataActions = useBookMetadataActions({
  editorBookId: bookDisplayEditorBookId, pendingRemoval, recoveryRequired,
  getRestoreRevision, notify,
});
const detailSession = useBookDetailSession({
  bookPanelBusy,
  isRecoveryRequired: () => recoveryRequired.value,
  getRestoreRevision,
  startReading: readerSession.startReading,
  closeDisplayEditor: () => { bookDisplayEditorBookId.value = null; },
  notify,
  errorText,
});
const {
  searchResultDetails,
  searchResultDetailsShelfBook,
  searchResultDetailsReadingBusy,
  searchResultSources,
} = detailSession;
const currentSearchResultDetails = computed(() => searchResultDetails.value ?? {
  result: openRequest?.kind === "search-result" ? openRequest.result : null,
  bookId: openRequest && openRequest.kind !== "search-result" ? openRequest.bookId : null,
  initialTab: openRequest && openRequest.kind !== "search-result" ? openRequest.initialTab ?? "catalog" : "info",
  error: pageResourceError.value,
});
const searchDetailSourceCandidates = computed(() => {
  const live = searchResultSources.value;
  const sourceIds = new Set(live.map(candidate => candidate.sourceId));
  const cached = openedBook.value && bookSourceSwitch.bookSourceSwitchBook?.id === openedBook.value.id
    ? bookSourceSwitch.bookSourceCandidateResults : [];
  return [...live, ...cached.filter(candidate => !sourceIds.has(candidate.sourceId))];
});
let openRequestStarted = false;
async function startOpenRequestOnce(): Promise<void> {
  if (!openRequest || openRequestStarted || disposed || closing) return;
  openRequestStarted = true;
  await detailSession.openRequest(openRequest);
}
function retainUnlistener(unlisten: UnlistenFn): void {
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}
async function setupPageEvents(): Promise<void> {
  try {
    retainUnlistener(await listen<AppBootstrap>("app-state-updated", async () => {
      const revision = getRestoreRevision();
      const detailsWereOpen = Boolean(searchResultDetails.value);
      pageResourceLoading.value = true;
      pageResourceError.value = "";
      clearOpenedBook();
      bookCacheActionGeneration += 1;
      bookChapterCacheClearBusy.value = false;
      pendingBookChapterCacheClear.value = null;
      bookChapterCacheClearError.value = "";
      bookDisplayEditorBookId.value = null;
      pendingBookProgressReset.value = null;
      pendingRemoval.value = null;
      try {
        if (disposed || closing || revision !== getRestoreRevision()) return;
        if (detailsWereOpen) await detailSession.retry();
        else await startOpenRequestOnce();
      } catch (error) {
        if (revision !== getRestoreRevision()) return;
        pageResourceError.value = errorText(error);
        clearOpenedBook();
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
    if (!disposed && !closing && revision === getRestoreRevision()) await startOpenRequestOnce();
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
  clearOpenedBook();
  try {
    await setupPageEvents();
    if (!disposed && !closing) {
      pageResourceError.value = "";
      pageResourceLoading.value = false;
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
  for (const unlisten of unlisteners.splice(0)) unlisten();
  void information?.cancel().catch(console.error);
  detailSession.clear({ preserveOpenedBook: readerVisible.value });
  bookSourceSwitch.closeBookSourceSwitch();
});
function closeBookDetails(): void {
  if (closing || router.currentRoute.value.name !== routeNames.bookDetail) return;
  closing = true;
  void information?.cancel().catch(console.error);
  detailSession.clear({ preserveOpenedBook: readerVisible.value });
  bookSourceSwitch.closeBookSourceSwitch();
  returnFromBookDetail(openRequest);
}
watch(() => {
  const activeDetails = searchResultDetails.value;
  return Boolean(activeDetails?.inShelf && activeDetails.bookId
    && !shelf.value.books.some((book) => book.id === activeDetails.bookId));
}, (bookWasRemoved) => {
  if (bookWasRemoved) closeBookDetails();
});
const recentCatalogChapters = computed(() => (openedBook.value?.chapters ?? [])
  .slice(-4).reverse());
const catalogAction = ref<BookAction | null>(null);
const catalogRefreshBusy = computed(() => catalogAction.value === "refresh");
const catalogCheckBusy = computed(() => catalogAction.value === "check");
let information: ReturnType<typeof moreInformation> | null = null;
const bookDetailTab = ref<"catalog" | "info" | "sources">("catalog");
const bookDetailMenuOpen = ref(false);
const bookDetailContentRef = ref<HTMLElement | null>(null);
function selectBookDetailTab(tab: "catalog" | "info" | "sources"): void {
  bookDetailTab.value = tab;
  bookDetailMenuOpen.value = false;
  void nextTick(() => {
    if (bookDetailContentRef.value) bookDetailContentRef.value.scrollTop = 0;
  });
}
watch(() => searchResultDetails.value, (details) => {
  bookDetailTab.value = details?.initialTab ?? "catalog";
  bookDetailMenuOpen.value = false;
}, { immediate: true });
watch(() => readerVisible.value, (visible) => {
  if (visible) {
    bookDetailMenuOpen.value = false;
    if (bookSourceSwitch.bookSourceSwitchInline) bookSourceSwitch.closeBookSourceSwitch();
  }
});
async function runCurrentBookAction(action: BookAction): Promise<void> {
  const book = openedBook.value;
  if (!book || recoveryRequired.value || catalogAction.value) return;
  const revision = getRestoreRevision();
  try {
    if (action === "download") {
      catalogAction.value = action;
      await Promise.all(book.chapters.filter(chapter => !chapter.volume).map(chapter => mainText(book.id, chapter.id, true)));
      if (disposed || revision !== getRestoreRevision()) return;
      await refreshOpenedBook(book.id);
      if (!disposed && revision === getRestoreRevision()) notify("整本缓存完成。");
      return;
    }
    if (!book.sourceId || !book.bookUrl) return;
    catalogAction.value = action;
    const previous = new Set(book.chapters.map(chapter => chapter.id));
    information = moreInformation(book.sourceId, book.bookUrl);
    await information;
    if (disposed || revision !== getRestoreRevision()) return;
    const updated = await refreshOpenedBook(book.id);
    const added = updated?.chapters.filter(chapter => !previous.has(chapter.id)).length ?? 0;
    notify(action === "check" ? (added ? `发现 ${added} 个新章节，目录已更新。` : "暂无新章节。") : "详情和目录已更新。");
  } catch (error) {
    if (!disposed && revision === getRestoreRevision()) notify(`操作失败：${errorText(error)}`, "error");
  } finally {
    catalogAction.value = null;
  }
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
        replaceOpenedBook({ ...bookAtStart, progress: { ...readerBook.progress } });
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
    if (warning || recoveryRequired.value) {
      pendingBookChapterCacheClear.value = null;
      if (openedBook.value?.id === pending.id) {
        replaceOpenedBook({ ...openedBook.value, chapters: openedBook.value.chapters.map((chapter) => ({ ...chapter, resource: undefined, pages: undefined, media: undefined })) });
      }
      if (warning) lockShelfBatchRecovery("需要重启应用完成缓存恢复", `《${pending.title}》的章节缓存已清除，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`);
      throw new Error(shelfBatchRecoveryMessage.value || "缓存清理需要完成恢复。");
    }

    pendingBookChapterCacheClear.value = null;
    let readError = "";
    try {
      await refreshOpenedBook(pending.id);
      if (!isCurrentOperation()) return;
    } catch (error) {
      if (!isCurrentOperation()) return;
      readError = errorText(error);
      if (openedBook.value?.id === pending.id) {
        replaceOpenedBook({
          ...openedBook.value,
          chapters: openedBook.value.chapters.map(chapter => ({ ...chapter, resource: undefined, pages: undefined, media: undefined })),
        });
      }
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
async function ensureInlineBookSources(): Promise<boolean> {
  const book = openedBook.value;
  if (!book || recoveryRequired.value || bookSourceSwitch.bookSourceSwitchBusy) return false;
  if (bookSourceSwitch.bookSourceSwitchOpen && bookSourceSwitch.bookSourceSwitchBook?.id === book.id) {
    bookSourceSwitch.bookSourceSwitchInline = true;
  } else {
    await bookSourceSwitch.openBookSourceSwitchForBook(book, undefined, true);
  }
  return !disposed && !closing && openedBook.value?.id === book.id && bookSourceSwitch.bookSourceSwitchOpen;
}
async function openBookSourceSwitch(chapterTarget?: { id: string; title: string }): Promise<void> {
  const book = openedBook.value;
  if (!book) return;
  selectBookDetailTab("sources");
  await bookSourceSwitch.openBookSourceSwitchForBook(book, chapterTarget, true);
}
async function searchMoreBookSources(): Promise<void> {
  if (await ensureInlineBookSources()) await bookSourceSwitch.startBookSourceCandidateSearch();
}
watch([bookDetailTab, () => openedBook.value?.id, () => bookSourceSwitch.bookSourceSwitchBusy, bookPanelBusy, readerVisible], async ([tab, , busy, loading, reading]) => {
  if (tab !== "sources" || busy || loading || reading || openedBook.value?.canChangeSource !== true) return;
  if (!await ensureInlineBookSources()) return;
  if (!bookSourceSwitch.bookSourceCandidateResults.length && !bookSourceSwitch.bookSourceSearchId && !bookSourceSwitch.bookSourceSearchBusy) {
    await bookSourceSwitch.startBookSourceCandidateSearch();
  }
});
async function chooseCachedBookSource(candidate: SearchBookResult): Promise<void> {
  if (!await ensureInlineBookSources()) return;
  await bookSourceSwitch.chooseBookSourceCandidate(candidate);
}
async function chooseSearchResultSource(candidate: SearchResultSource): Promise<void> {
  if (openedBook.value && !bookPanelBusy.value) {
    await chooseCachedBookSource(candidate);
  } else {
    await detailSession.selectSearchResultSource(candidate);
  }
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
              <button type="button" class="book-detail-icon-button" aria-label="更多书籍操作" :aria-expanded="bookDetailMenuOpen"
                @click.stop="bookDetailMenuOpen = !bookDetailMenuOpen"><PrototypeIcon name="more"/></button>
              <div v-if="bookDetailMenuOpen" class="book-detail-menu" @click.stop>
                <button v-if="!searchResultDetailsShelfBook && currentSearchResultDetails.result"
                  class="book-detail-menu-item"
                  :disabled="bookPanelBusy || recoveryRequired || searchResultDetailsReadingBusy"
                  @click="bookDetailMenuOpen = false; detailSession.addSearchResultFromDetails()">
                  <PrototypeIcon name="plus"/><span>加入书架</span>
                </button>
                <template v-if="searchResultDetailsShelfBook">
                <button class="book-detail-menu-item"
                  :disabled="recoveryRequired"
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
                  @click="bookDetailMenuOpen = false; runCurrentBookAction('check')">
                  <PrototypeIcon name="search"/><span>{{ catalogCheckBusy ? '正在检查…' : '检查更新' }}</span>
                </button>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId" class="book-detail-menu-item"
                  :disabled="recoveryRequired"
                  @click="bookDetailMenuOpen = false; runCurrentBookAction('download')">
                  <PrototypeIcon name="download"/><span>缓存全部{{ openedBook.mediaType === 'video' ? '剧集' : openedBook.mediaType === 'audio' ? '节目' : '章节' }}</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button class="book-detail-menu-item"
                  :disabled="bookProgressResetBusy || bookPanelBusy || bookInfoRefreshBusy || recoveryRequired"
                  @click="pendingBookProgressReset = { id: openedBook.id, title: openedBook.title }; bookProgressResetError = ''; bookDetailMenuOpen = false">
                  <PrototypeIcon name="history"/><span>重置{{ openedBook.mediaType === 'video' ? '观看' : openedBook.mediaType === 'audio' ? '收听' : '阅读' }}进度</span>
                </button>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId"
                  class="book-detail-menu-item"
                  :disabled="bookChapterCacheClearBusy || bookPanelBusy || bookInfoRefreshBusy || bookProgressResetBusy || bookSourceSwitch.bookSourceSwitchBusy || recoveryRequired"
                  @click="pendingBookChapterCacheClear = { id: openedBook.id, title: openedBook.title }; bookChapterCacheClearError = ''; bookDetailMenuOpen = false">
                  <PrototypeIcon name="trash"/><span>{{ bookChapterCacheClearBusy ? '清除中…' : '清除正文缓存' }}</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button v-if="openedBook.canChangeSource === true && openedBook.sourceId" class="book-detail-menu-item"
                  :disabled="catalogRefreshBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; runCurrentBookAction('refresh')">
                  <PrototypeIcon name="refresh"/><span>{{ catalogRefreshBusy ? '正在更新目录…' : '更新目录' }}</span>
                </button>
                <button v-if="openedBook.canChangeSource === true"
                  class="book-detail-menu-item" :disabled="bookPanelBusy || recoveryRequired"
                  @click="bookDetailMenuOpen = false; openBookSourceSwitch()">
                  <PrototypeIcon name="source-switch"/><span>更换来源</span>
                </button>
                <div class="book-detail-menu-divider"></div>
                <button class="book-detail-menu-item book-detail-menu-item--danger"
                  :disabled="recoveryRequired"
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
          <BookSourcesPanel v-if="bookDetailTab === 'sources' && currentSearchResultDetails.result"
            :book="openedBook || currentSearchResultDetails.result" :candidates="searchDetailSourceCandidates" :session="bookSourceSwitch"
            :loading="bookPanelBusy" :locked="recoveryRequired" :error="currentSearchResultDetails.error"
            :can-refresh="canRefreshOpenedBookInfo" :refresh-busy="bookInfoRefreshBusy"
            :can-check="!!searchResultDetailsShelfBook && !!openedBook" :check-busy="catalogCheckBusy"
            @select="chooseSearchResultSource" @search="searchMoreBookSources" @refresh="bookDetailActions.refreshOpenedBookInfo"
            @check="runCurrentBookAction('check')" @retry="retryBookDetails" />
          <section v-else-if="!openedBook && currentSearchResultDetails.result && bookDetailTab === 'info'" class="book-info-view">
            <div class="detail-hero">
              <img v-if="currentSearchResultDetails.result.coverSrc" :src="currentSearchResultDetails.result.coverSrc" decoding="async"/>
              <div v-else class="detail-cover-fallback">{{ currentSearchResultDetails.result.title.slice(0, 1) }}</div>
              <div class="detail-title-copy">
                <h2>{{ currentSearchResultDetails.result.title }}</h2>
                <p class="detail-author">{{ currentSearchResultDetails.result.author || '作者未知' }}</p>
                <p>{{ currentSearchResultDetails.result.sourceName }}</p>
                <button type="button" class="button secondary" @click="selectBookDetailTab('sources')">更换来源（{{ searchResultSources.length }}）</button>
              </div>
            </div>
            <p v-if="bookPanelBusy || pageResourceLoading" class="detail-loading-notice" role="status">正在加载详情和目录，可在来源页切换其他源。</p>
            <p v-if="currentSearchResultDetails.error">{{ currentSearchResultDetails.error }}
              <button type="button" class="text-button" @click="retryBookDetails">重试</button>
            </p>
            <section class="detail-section detail-intro"><header><h3>简介</h3></header>
              <p>{{ currentSearchResultDetails.result.intro || '暂无简介。' }}</p>
            </section>
          </section>
          <div v-else-if="bookPanelBusy || pageResourceLoading" class="catalog-empty">正在读取内容详情…
            <button v-if="currentSearchResultDetails.result" type="button" class="text-button" @click="selectBookDetailTab('sources')">更换来源</button>
          </div>
          <div v-else-if="currentSearchResultDetails.error" class="catalog-empty">
            {{ currentSearchResultDetails.error }}
            <button v-if="currentSearchResultDetails.result" class="button secondary small" @click="retryBookDetails">重试</button>
            <button v-else-if="currentSearchResultDetails.bookId" class="button secondary small" @click="retryBookDetails">重试</button>
          </div>
          <template v-else-if="openedBook">
            <BookCatalogPanel v-if="bookDetailTab === 'catalog'" :key="openedBook.id" :book="openedBook"
              :busy="searchResultDetailsReadingBusy" :locked="recoveryRequired"
              :can-download="!!searchResultDetailsShelfBook && openedBook.canChangeSource === true && !!openedBook.sourceId"
              :can-replace="!!searchResultDetailsShelfBook && openedBook.canChangeSource === true && !isMediaReaderBook(openedBook)"
              @select="detailSession.startReading" @download="runCurrentBookAction('download')" @replace="openBookSourceSwitch" />
            <BookSourcesPanel v-else-if="bookDetailTab === 'sources'" :book="openedBook" :candidates="bookSourceSwitch.bookSourceCandidateResults" :session="bookSourceSwitch"
              :locked="recoveryRequired" :can-refresh="canRefreshOpenedBookInfo" :refresh-busy="bookInfoRefreshBusy"
              :can-check="!!searchResultDetailsShelfBook && openedBook.canChangeSource === true" :check-busy="catalogCheckBusy"
              @select="chooseCachedBookSource" @search="searchMoreBookSources" @refresh="bookDetailActions.refreshOpenedBookInfo" @check="runCurrentBookAction('check')" />

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
                      : bookContentType(openedBook) === 'image' ? '图片'
                      : bookContentType(openedBook) === 'pdf' ? 'PDF'
                      : bookContentType(openedBook) === 'html' ? 'HTML' : '文本' }}
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
                      :disabled="bookPanelBusy || recoveryRequired || searchResultDetailsReadingBusy"
                      @click="detailSession.addSearchResultFromDetails()">
                      加入书架
                    </button>
                  </div>
                </div>
                <aside class="prototype-overview-progress">
                  <div>
                    <small>{{ openedBook.mediaType === 'video' ? '观看进度' : openedBook.mediaType === 'audio' ? '收听进度' : '阅读进度' }}</small>
                    <strong v-if="hasReadingProgress(openedBook.progress)">
                      第 {{ (openedBook.progress?.chapterIndex ?? 0) + 1 }}
                      {{ openedBook.mediaType === 'video' ? '集' : openedBook.mediaType === 'audio' ? '期' : bookContentType(openedBook) === 'image' ? '话' : bookContentType(openedBook) === 'pdf' ? '页' : '章' }}
                      <template v-if="openedBook.mediaType"> · {{ progressTime(openedBook.progress?.offset) }}</template>
                      <template v-else> · {{ readingProgressPercent(openedBook.progress, openedBook.chapterCount || openedBook.chapters.length) }}%</template>
                    </strong>
                    <strong v-else>尚未开始</strong>
                  </div>
                  <div v-if="!openedBook.mediaType" class="detail-reading-track"

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
          :locked="recoveryRequired"
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
        <p>章节目录、已缓存正文、源、书签和阅读历史都会保留。</p>
        <p v-if="bookProgressResetError" class="source-identity-confirmation">{{ bookProgressResetError }}</p>
        <div class="modal-actions">
          <button class="button secondary" data-back-action :disabled="bookProgressResetBusy" @click="pendingBookProgressReset = null">取消</button>
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
          <button class="button secondary" data-back-action :disabled="bookChapterCacheClearBusy" @click="pendingBookChapterCacheClear = null">取消</button>
          <button class="button danger" :disabled="bookChapterCacheClearBusy || recoveryRequired" @click="clearOpenedBookChapterCache">
            {{ bookChapterCacheClearBusy ? '正在清除…' : '确认清除缓存' }}
          </button>
        </div>
      </article>
    </section>

    <section
      v-if="pendingRemoval"
      class="modal-backdrop"
      @click.self="pendingRemoval = null"
    >
      <article class="app-modal confirm-modal">
        <div class="confirm-symbol">⌫</div>
        <h2>从书架移除这本书？</h2>
        <p>《{{ pendingRemoval.title }}》及其本地书籍数据、阅读进度和书签都会被移除。</p>
        <div class="modal-actions">
          <button class="button secondary" data-back-action @click="pendingRemoval = null">保留</button>
          <button class="button danger" :disabled="recoveryRequired" @click="bookMetadataActions.removeBookFromShelf">
            确认移除
          </button>
        </div>
      </article>
    </section>

    <BookSourceSwitchView v-if="!readerVisible" :session="bookSourceSwitch" :locked="recoveryRequired" />

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
  font-size: var(--ui-font-body);
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

.book-info-view {
  display: block;
}
.detail-loading-notice { margin: 16px 0; padding: 12px 14px; border-radius: 12px; color: var(--app-accent-ink); background: var(--app-accent-soft); line-height: 1.7; }

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
  font-size: var(--ui-font-caption);
}

.detail-type-label {
  color: var(--app-accent);
  font-size: var(--ui-font-body);
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
  font-size: var(--ui-font-caption);
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
  font-size: var(--ui-font-caption);
}

.prototype-overview-progress strong {
  font-size: var(--ui-font-caption);
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
  font-size: var(--ui-font-caption);
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

.book-detail-fact strong {
  overflow: hidden;
  font-size: var(--ui-font-caption);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.detail-section {
  display: grid;
  gap: 8px;
  margin-top: 28px;
}

.detail-section header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.detail-section h3 {
  margin: 0;
  font-size: var(--ui-font-body);
}

.detail-section p {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  line-height: 1.85;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
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
    padding: 22px 13px calc(22px + var(--safe-bottom));
  }

  .book-detail-content--catalog {
    padding: 0 13px var(--safe-bottom);
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
    font-size: var(--ui-font-title);
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
    font-size: var(--ui-font-body);
  }

  .prototype-catalog-tools {
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
    margin: 0 0 12px;
    padding: 10px 12px 9px;
    box-shadow: 0 5px 12px rgb(32 34 42 / 4%);
  }
.catalog-row-wrap {
    grid-template-columns: minmax(0, 1fr) 34px;
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
    font-size: var(--ui-font-body);
  }

  .catalog-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }
}

</style>
