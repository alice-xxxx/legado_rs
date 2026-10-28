<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton, Popover as KPopover, Button as KButton, Segmented as KSegmented, SegmentedButton as KSegmentedButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sources } from "../sources/sourcesStore";
import { clearBookChapterCache as clearBookChapterCacheApi } from "../../api/books";
import { moreInformation } from "../../api/moreInformation";
import { mainText } from "../../api/mainText";
import type { AppBootstrap, BookResource, SearchBookResult, SearchResultSource } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import router, { routeNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
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
  applyBookInformation: readerSession.applyBookInformation,
  closeDisplayEditor: () => { bookDisplayEditorBookId.value = null; },
  notify,
  errorText,
});
const {
  searchResultDetails,
  searchResultDetailsShelfBook,
  searchResultDetailsReadingBusy,
  searchResultSources,
  informationRefreshBusy,
  informationRefreshError,
} = detailSession;
const currentSearchResultDetails = computed(() => searchResultDetails.value ?? {
  result: openRequest?.kind === "search-result" ? openRequest.result : null,
  bookId: openRequest && openRequest.kind !== "search-result" ? openRequest.bookId : null,
  initialTab: openRequest && openRequest.kind !== "search-result" ? openRequest.initialTab ?? "info" : "info",
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
const recentCatalogChapters = computed(() => (openedBook.value?.chapters ?? [])
  .slice(-4).reverse());
const catalogAction = ref<BookAction | null>(null);
const catalogRefreshBusy = computed(() => catalogAction.value === "refresh");
const catalogCheckBusy = computed(() => catalogAction.value === "check");
let information: ReturnType<typeof moreInformation> | null = null;
const bookDetailTab = ref<"catalog" | "info" | "sources">("info");
const bookDetailMenuOpen = ref(false);
const bookDetailMenuElement = ref<HTMLElement | null>(null);
useOutsidePointerDismiss(
  () => bookDetailMenuOpen.value,
  target => target instanceof Node && Boolean(bookDetailMenuElement.value?.contains(target) || target instanceof Element && target.closest(".book-detail-menu")),
  () => { bookDetailMenuOpen.value = false; },
);
const bookOperationBusy = computed(() => bookPanelBusy.value || informationRefreshBusy.value || bookInfoRefreshBusy.value
  || catalogAction.value !== null || bookProgressResetBusy.value || bookChapterCacheClearBusy.value || bookSourceSwitch.bookSourceSwitchBusy || bookSourceSwitch.bookSourceCheckBusy);
const canClearChapterCache = computed(() => openedBook.value?.canChangeSource === true && !!openedBook.value.sourceId
  && openedBook.value.chapters.some(chapter => chapter.resource || chapter.pages?.length || chapter.media));
const bookDetailContentRef = ref<HTMLElement | null>(null);
function selectBookDetailTab(tab: "catalog" | "info" | "sources"): void {
  bookDetailTab.value = tab;
  bookDetailMenuOpen.value = false;
  void nextTick(() => {
    if (bookDetailContentRef.value) bookDetailContentRef.value.scrollTop = 0;
  });
}
watch(() => searchResultDetails.value, (details) => {
  bookDetailTab.value = details?.initialTab ?? "info";
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
  if (!book || recoveryRequired.value || bookOperationBusy.value) return;
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

function toggleBookShelf(): void {
  const book = openedBook.value;
  if (!book || bookPanelBusy.value || recoveryRequired.value || searchResultDetailsReadingBusy.value) return;
  if (searchResultDetailsShelfBook.value) pendingRemoval.value = { id: book.id, title: book.title };
  else void detailSession.addSearchResultFromDetails(searchDetailSourceCandidates.value);
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
  await ensureInlineBookSources();
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
      <SubpageHeader back-label="返回" @back="closeBookDetails"><template #title><KSegmented strong rounded class="max-w-sm" aria-label="书籍详情">
          <KSegmentedButton rounded :active="bookDetailTab === 'info'" class="gap-2" type="button" :class="{ active: bookDetailTab === 'info' }" :aria-pressed="bookDetailTab === 'info'"
            @click="selectBookDetailTab('info')">概览</KSegmentedButton>
          <KSegmentedButton rounded :active="bookDetailTab === 'catalog'" class="gap-2" type="button" :class="{ active: bookDetailTab === 'catalog' }"
            :aria-pressed="bookDetailTab === 'catalog'" @click="selectBookDetailTab('catalog')">{{ openedBook?.mediaType
              === 'video' ? '剧集' : openedBook?.mediaType === 'audio' ? '节目' : '目录' }}</KSegmentedButton>
          <KSegmentedButton rounded :active="bookDetailTab === 'sources'" class="gap-2" type="button" :class="{ active: bookDetailTab === 'sources' }"
            :aria-pressed="bookDetailTab === 'sources'" @click="selectBookDetailTab('sources')">来源</KSegmentedButton>
        </KSegmented></template>
<template #actions><div v-if="openedBook" class="book-detail-toolbar-actions">
          <div ref="bookDetailMenuElement" class="book-detail-menu-wrap">
            <KButton rounded inline clear type="button" class="book-detail-icon-button gap-2 w-full justify-start text-current" aria-label="更多书籍操作"
              :aria-expanded="bookDetailMenuOpen" @click.stop="bookDetailMenuOpen = !bookDetailMenuOpen">
              <PrototypeIcon name="more" />
            </KButton>

          </div>
        </div></template></SubpageHeader>
<KPopover v-if="bookDetailMenuOpen && openedBook" opened :backdrop="false" target=".book-detail-menu-wrap" class="book-detail-menu w-60 max-w-[calc(100vw-32px)]">
  <div class="p-2">
<KButton rounded inline clear class="book-detail-menu-item gap-2 w-full justify-start text-current" :disabled="bookOperationBusy || recoveryRequired"
                @click="bookDetailMenuOpen = false; bookMetadataActions.openBookDisplayEditor()">
                <PrototypeIcon name="edit" /><span>编辑显示信息</span>
              </KButton>
<KButton rounded inline clear v-if="canRefreshOpenedBookInfo" class="book-detail-menu-item gap-2 w-full justify-start text-current"
                :disabled="bookOperationBusy || recoveryRequired"
                @click="bookDetailMenuOpen = false; bookDetailActions.refreshOpenedBookInfo()">
                <PrototypeIcon name="refresh" /><span>{{ bookInfoRefreshBusy ? '刷新中…' : '刷新信息' }}</span>
              </KButton>
<KButton rounded inline clear v-if="canRefreshOpenedBookInfo" class="book-detail-menu-item gap-2 w-full justify-start text-current"
                :disabled="bookOperationBusy || recoveryRequired"
                @click="bookDetailMenuOpen = false; runCurrentBookAction('refresh')">
                <PrototypeIcon name="refresh" /><span>{{ catalogRefreshBusy ? '正在更新目录…' : '更新目录' }}</span>
              </KButton><div v-if="hasReadingProgress(openedBook.progress) || canClearChapterCache"
                class="book-detail-menu-divider"></div><KButton rounded inline clear v-if="hasReadingProgress(openedBook.progress)" class="book-detail-menu-item gap-2 w-full justify-start text-current"
                :disabled="bookOperationBusy || recoveryRequired"
                @click="pendingBookProgressReset = { id: openedBook.id, title: openedBook.title }; bookProgressResetError = ''; bookDetailMenuOpen = false">
                <PrototypeIcon name="history" /><span>重置{{ openedBook.mediaType === 'video' ? '观看' :
                  openedBook.mediaType === 'audio' ? '收听' : '阅读' }}进度</span>
              </KButton>
<KButton rounded inline clear v-if="canClearChapterCache" class="book-detail-menu-item gap-2 w-full justify-start text-current"
                :disabled="bookOperationBusy || recoveryRequired"
                @click="pendingBookChapterCacheClear = { id: openedBook.id, title: openedBook.title }; bookChapterCacheClearError = ''; bookDetailMenuOpen = false">
                <PrototypeIcon name="trash" /><span>{{ bookChapterCacheClearBusy ? '清除中…' : '清除正文缓存' }}</span>
              </KButton>
  </div>
</KPopover>

      <p v-if="informationRefreshBusy || informationRefreshError" class="status-notice detail-refresh-status"
        :class="{ 'is-error': informationRefreshError }" role="status">
        {{ informationRefreshError || '正在后台刷新详情，已缓存内容可继续使用。' }}
      </p>

      <main ref="bookDetailContentRef" class="book-detail-content"
        :class="{ 'book-detail-content--catalog': bookDetailTab === 'catalog', 'book-detail-content--sources': bookDetailTab === 'sources' }"
        @click="bookDetailMenuOpen = false">
        <BookSourcesPanel v-if="bookDetailTab === 'sources' && currentSearchResultDetails.result"
          :book="openedBook || currentSearchResultDetails.result" :candidates="searchDetailSourceCandidates"
          :session="bookSourceSwitch" :loading="bookPanelBusy" :locked="recoveryRequired"
          :error="currentSearchResultDetails.error" :can-refresh="canRefreshOpenedBookInfo"
          :refresh-busy="bookInfoRefreshBusy || informationRefreshBusy" :can-check="canRefreshOpenedBookInfo"
          :check-busy="catalogCheckBusy || informationRefreshBusy" @select="chooseSearchResultSource"
          @search="searchMoreBookSources" @refresh="bookDetailActions.refreshOpenedBookInfo"
          @check="runCurrentBookAction('check')" @retry="retryBookDetails" />
        <section v-else-if="!openedBook && currentSearchResultDetails.result && bookDetailTab === 'info'"
          class="book-info-view">
          <div class="detail-hero">
            <img v-if="currentSearchResultDetails.result.coverSrc" :src="currentSearchResultDetails.result.coverSrc"
              decoding="async" alt="封面" />
            <div v-else class="detail-cover-fallback">{{ currentSearchResultDetails.result.title.slice(0, 1) }}</div>
            <div class="detail-title-copy">
              <h2>{{ currentSearchResultDetails.result.title }}</h2>
              <p class="detail-author">{{ currentSearchResultDetails.result.author || '作者未知' }}</p>
              <p class="detail-source-name">{{ currentSearchResultDetails.result.sourceName }}</p>
            </div>
            <div class="detail-hero-actions">
              <KButton rounded inline tonal type="button" class="gap-2" @click="selectBookDetailTab('sources')">更换来源（{{
                searchResultSources.length }}）</KButton>
            </div>
          </div>
          <p v-if="bookPanelBusy || pageResourceLoading" class="status-notice" role="status">正在加载详情和目录，可在来源页切换其他源。</p>
          <p v-if="currentSearchResultDetails.error" class="status-notice is-error">{{ currentSearchResultDetails.error
            }}
            <KButton rounded inline clear type="button" class="gap-2" @click="retryBookDetails">重试</KButton>
          </p>
          <section class="detail-section detail-intro">
            <header>
              <h3>简介</h3>
            </header>
            <p>{{ currentSearchResultDetails.result.intro || '暂无简介。' }}</p>
          </section>
        </section>
        <div v-else-if="bookPanelBusy || pageResourceLoading" class="page-empty-state" role="status">正在读取内容详情…
          <KButton rounded inline clear v-if="currentSearchResultDetails.result" type="button" class="gap-2"
            @click="selectBookDetailTab('sources')">更换来源</KButton>
        </div>
        <div v-else-if="currentSearchResultDetails.error" class="page-empty-state detail-error-notice">
          {{ currentSearchResultDetails.error }}
          <KButton rounded inline tonal small v-if="currentSearchResultDetails.result" class="gap-2"
            @click="retryBookDetails">重试</KButton>
          <KButton rounded inline tonal small v-else-if="currentSearchResultDetails.bookId" class="gap-2"
            @click="retryBookDetails">重试</KButton>
        </div>
        <template v-else-if="openedBook">
          <BookCatalogPanel v-if="bookDetailTab === 'catalog'" :key="openedBook.id" :book="openedBook"
            :busy="searchResultDetailsReadingBusy" :locked="recoveryRequired"
            :can-download="canRefreshOpenedBookInfo && !bookOperationBusy"
            :can-replace="openedBook.canChangeSource === true && !isMediaReaderBook(openedBook)"
            @select="detailSession.startReading" @download="runCurrentBookAction('download')"
            @replace="openBookSourceSwitch" />
          <BookSourcesPanel v-else-if="bookDetailTab === 'sources'" :book="openedBook"
            :candidates="bookSourceSwitch.bookSourceCandidateResults" :session="bookSourceSwitch"
            :locked="recoveryRequired" :can-refresh="canRefreshOpenedBookInfo"
            :refresh-busy="bookInfoRefreshBusy || informationRefreshBusy" :can-check="canRefreshOpenedBookInfo"
            :check-busy="catalogCheckBusy || informationRefreshBusy" @select="chooseCachedBookSource"
            @search="searchMoreBookSources" @refresh="bookDetailActions.refreshOpenedBookInfo"
            @check="runCurrentBookAction('check')" />

          <section v-else class="book-info-view">
            <div class="detail-hero">
              <img v-if="openedBook.coverSrc || currentSearchResultDetails.result?.coverSrc"
                :src="openedBook.coverSrc || currentSearchResultDetails.result?.coverSrc" decoding="async" alt="封面" />
              <div v-else class="detail-cover-fallback">{{ openedBook.title.slice(0, 1) }}</div>
              <div class="detail-title-copy">
                <h2>{{ openedBook.title }}</h2>
                <p class="detail-author">{{ openedBook.author || '作者未知' }}</p>
              </div>
              <div class="detail-hero-actions">
                <KButton rounded inline class="gap-2" type="button"
                  :disabled="bookPanelBusy || searchResultDetailsReadingBusy || recoveryRequired || !(openedBook.chapterCount || openedBook.chapters?.length)"
                  @click="detailSession.startReading()">
                  <PrototypeIcon
                    :name="openedBook.mediaType === 'video' ? 'play' : openedBook.mediaType === 'audio' ? 'speaker' : 'book'" />
                  {{ openedBook.mediaType === 'video'
                    ? hasReadingProgress(openedBook.progress) ? '继续观看' : '开始观看'
                    : openedBook.mediaType === 'audio'
                      ? hasReadingProgress(openedBook.progress) ? '继续收听' : '开始收听'
                      : hasReadingProgress(openedBook.progress) ? '继续阅读' : '开始阅读' }}
                </KButton>
                <KButton rounded inline tonal type="button" class="gap-2"
                  :disabled="bookPanelBusy || recoveryRequired || searchResultDetailsReadingBusy"
                  @click="toggleBookShelf">
                  {{ searchResultDetailsShelfBook ? '移出' : '加入书架' }}
                </KButton>
              </div>
            </div>
            <button type="button" class="prototype-overview-progress" aria-label="查看目录和阅读位置"
              @click="selectBookDetailTab('catalog')">
              <span class="detail-progress-copy">
                <small>{{ openedBook.mediaType === 'video' ? '观看进度' : openedBook.mediaType === 'audio' ? '收听进度' : '阅读进度'
                  }}</small>
                <strong v-if="hasReadingProgress(openedBook.progress)">
                  第 {{ (openedBook.progress?.chapterIndex ?? 0) + 1 }}
                  {{ openedBook.mediaType === 'video' ? '集' : openedBook.mediaType === 'audio' ? '期' :
                    bookContentType(openedBook) === 'image' ? '话' : bookContentType(openedBook) === 'pdf' ? '页' : '章' }}
                  <template v-if="openedBook.mediaType"> · {{ progressTime(openedBook.progress?.offset) }}</template>
                  <template v-else> · {{ readingProgressPercent(openedBook.progress, openedBook.chapterCount ||
                    openedBook.chapters.length) }}%</template>
                </strong>
                <strong v-else>尚未开始</strong>
              </span>
              <span v-if="!openedBook.mediaType" class="detail-reading-track">
                <span
                  :style="{ width: `${readingProgressPercent(openedBook.progress, openedBook.chapterCount || openedBook.chapters.length)}%` }"></span>
              </span>
              <PrototypeIcon class="detail-progress-arrow" name="right" />
            </button>
            <div class="book-detail-facts">
              <button type="button" class="book-detail-fact detail-fact-link" @click="selectBookDetailTab('catalog')">
                <span><small>最新章节</small><strong :title="openedBook.latestChapter || recentCatalogChapters[0]?.title">{{
                  openedBook.latestChapter || recentCatalogChapters[0]?.title || '暂无目录信息' }}</strong></span>
                <PrototypeIcon name="right" />
              </button>
              <div class="book-detail-fact">
                <span><small>{{ openedBook.wordCount ? '字数' : '目录总数' }}</small><strong>{{ openedBook.wordCount ||
                  `${openedBook.chapterCount || openedBook.chapters.length} ${openedBook.mediaType === 'video' ? '集' :
                    openedBook.mediaType === 'audio' ? '期' : bookContentType(openedBook) === 'image' ? '话' :
                    bookContentType(openedBook) === 'pdf' ? '页' : '章'}` }}</strong></span>
              </div>
              <button type="button" class="book-detail-fact detail-fact-link" @click="selectBookDetailTab('sources')">
                <span><small>当前来源</small><strong :title="openedBook.sourceName">{{ openedBook.sourceName || '本地内容'
                    }}</strong></span>
                <PrototypeIcon name="right" />
              </button>
              <div class="book-detail-fact">
                <span><small>分类</small><strong class="detail-kind">{{ openedBook.kind || (openedBook.mediaType ===
                  'video' ? '视频' :
                  openedBook.mediaType === 'audio' ? '音频' : '阅读内容') }}</strong></span>
              </div>
            </div>
            <section class="detail-section detail-intro">
              <header>
                <h3>简介</h3>
              </header>
              <p>{{ openedBook.intro || '暂无简介。' }}</p>
            </section>
          </section>
        </template>
        <div v-else class="page-empty-state">暂无可显示的内容信息。</div>
      </main>



      <BookDisplayMetadataEditor v-if="openedBook && bookDisplayEditorBookId === openedBook.id" :key="openedBook.id"
        :book="openedBook" :locked="recoveryRequired" :return-label="bookDetailTab === 'catalog' ? '返回目录' : '返回概览'"
        @close="bookMetadataActions.closeBookDisplayEditor" @save="bookMetadataActions.saveBookDisplayMetadata" />
    </article>
  </section>

  <section v-if="pendingBookProgressReset" class="modal-backdrop"
    @click.self="!bookProgressResetBusy && (pendingBookProgressReset = null)">
    <KDialog title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]" opened :backdrop="false" class="confirm-modal w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h2 id="book-reset-progress-title">重置阅读进度？</h2></template>
<p>将清除《{{ pendingBookProgressReset.title }}》的阅读位置，从第一章重新开始。若当前阅读器打开了这本书，会先保存当前位置再关闭阅读器。</p>
<p>章节目录、已缓存正文、源、书签和阅读历史都会保留。</p>
<p v-if="bookProgressResetError" class="source-identity-confirmation">{{ bookProgressResetError }}</p><template #buttons><KDialogButton data-back-action :disabled="bookProgressResetBusy" @click="pendingBookProgressReset = null">取消</KDialogButton>
<KDialogButton :disabled="bookProgressResetBusy || recoveryRequired" @click="bookDetailActions.resetOpenedBookProgress" class="k-color-brand-danger" strong>{{ bookProgressResetBusy ? '正在重置…' : '确认重置进度' }}</KDialogButton></template>
</KDialog>
  </section>

  <section v-if="pendingBookChapterCacheClear" class="modal-backdrop"
    @click.self="!bookChapterCacheClearBusy && (pendingBookChapterCacheClear = null)">
    <KDialog title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]" opened :backdrop="false" class="confirm-modal w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h2 id="book-clear-chapter-cache-title">清除章节缓存？</h2></template>
<p>将清除《{{ pendingBookChapterCacheClear.title }}》已缓存的章节正文。之后可以重新下载正文。</p>
<p>阅读进度、章节目录、封面和书签都会保留。<span
          v-if="readingBook?.id === pendingBookChapterCacheClear.id">当前阅读器会先保存阅读位置并关闭，避免继续显示已清除的正文。</span></p>
<p v-if="bookChapterCacheClearError" class="source-identity-confirmation">{{ bookChapterCacheClearError }}</p><template #buttons><KDialogButton data-back-action :disabled="bookChapterCacheClearBusy" @click="pendingBookChapterCacheClear = null">取消</KDialogButton>
<KDialogButton :disabled="bookChapterCacheClearBusy || recoveryRequired" @click="clearOpenedBookChapterCache" class="k-color-brand-danger" strong>{{ bookChapterCacheClearBusy ? '正在清除…' : '确认清除缓存' }}</KDialogButton></template>
</KDialog>
  </section>

  <section v-if="pendingRemoval" class="modal-backdrop" @click.self="pendingRemoval = null">
    <KDialog title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]" opened :backdrop="false" class="confirm-modal w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h2 id="book-remove-title">从书架移除这本书？</h2></template>
<p>将《{{ pendingRemoval.title }}》从书架移除，详情缓存、阅读进度和书签都会保留。</p><template #buttons><KDialogButton data-back-action @click="pendingRemoval = null">保留</KDialogButton>
<KDialogButton :disabled="recoveryRequired" @click="bookMetadataActions.removeBookFromShelf" class="k-color-brand-danger" strong>
          确认移除
        </KDialogButton></template>
</KDialog>
  </section>

  <BookSourceSwitchView v-if="!readerVisible" :session="bookSourceSwitch" :locked="recoveryRequired" />

</template>

<style scoped>
@layer components {
.book-detail-page {
  display: flex;
  justify-content: center;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  overflow: hidden;
  background: var(--app-background);
}

.book-detail-panel {
  display: flex;
  flex-direction: column;
  width: min(var(--app-content-width), 100%);
  height: 100%;
  min-height: 0;
  background: var(--app-background);
}

.book-detail-icon-button {
  display: grid;
  place-items: center;
  width: 44px;
  height: 44px;
}

.book-detail-menu-wrap {
  position: relative;
}

.book-detail-menu-item {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 44px;
  padding: 8px 10px;
  text-align: left;
}

.book-detail-menu-item :deep(svg) {
  width: 18px;
  height: 18px;
  color: var(--app-muted);
}

.book-detail-menu-divider {
  height: 1px;
  margin: 6px 10px;
  background: var(--app-line);
}

.book-detail-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: var(--ui-space-6) var(--ui-page-gutter) calc(max(var(--ui-space-6), var(--safe-bottom)) + var(--ui-playback-inset));
  color: var(--app-text);
  overscroll-behavior: contain;
}

.book-detail-content--catalog {
  overflow: hidden;
  padding: 0 0 var(--safe-bottom);
}

.book-detail-content--sources {
  padding-top: var(--ui-space-2);
}

.detail-refresh-status {
  flex: none;
  margin: 0;
  border-radius: 0;
}

.detail-error-notice {
  color: var(--app-danger);
}

.detail-hero {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  align-items: start;
  column-gap: 20px;
  row-gap: 16px;
}

.detail-hero>img, .detail-cover-fallback {
  display: grid;
  grid-row: 1 / 3;
  width: 112px;
  height: 150px;
  overflow: hidden;
  border-radius: 8px;
  object-fit: cover;
}

.detail-cover-fallback {
  place-items: center;
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
  font-size: var(--ui-font-title);
  font-weight: 600;
}

.detail-title-copy {
  display: grid;
  align-content: start;
  gap: 8px;
  min-width: 0;
  padding-top: 4px;
}

.detail-title-copy h2 {
  margin: 0;
  font-size: var(--ui-font-title);
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.detail-author, .detail-source-name {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-body);
  overflow-wrap: anywhere;
}

.detail-hero-actions {
  display: flex;
  flex-wrap: wrap;
  grid-column: 2;
  align-self: end;
  gap: 6px;
}

.detail-hero-actions :deep(svg) {
  width: 16px;
  height: 16px;
}

.prototype-overview-progress {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 12px;
  width: 100%;
  margin-top: 24px;
  padding: 16px 0;
  border-bottom: 1px solid var(--app-line);
  text-align: left;
  cursor: pointer;
}

.detail-progress-copy {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 8px 16px;
  min-width: 0;
}

.prototype-overview-progress small {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.prototype-overview-progress strong {
  font-size: var(--ui-font-body);
  font-weight: 600;
}

.detail-progress-arrow {
  grid-column: 2;
  grid-row: 1;
  width: 18px;
  height: 18px;
  color: var(--app-muted);
}

.detail-reading-track {
  grid-column: 1 / -1;
  grid-row: 2;
  height: 4px;
  overflow: hidden;
  border-radius: 4px;
  background: var(--app-line);
}

.detail-reading-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--app-accent);
}

.book-detail-facts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  column-gap: 24px;
  margin-top: 16px;
}

.book-detail-fact {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-width: 0;
  min-height: 76px;
  padding-block: 14px;
  border-bottom: 1px solid var(--app-line);
  text-align: left;
}

.book-detail-fact>span {
  display: grid;
  gap: 5px;
  min-width: 0;
}

.book-detail-fact small {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.book-detail-fact strong {
  overflow: hidden;
  font-size: var(--ui-font-body);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.book-detail-fact .detail-kind {
  white-space: normal;
  overflow-wrap: anywhere;
  font-weight: 400;
  line-height: 1.6;
}

.book-detail-fact :deep(svg) {
  width: 16px;
  height: 16px;
  color: var(--app-muted);
}

.detail-fact-link {
  cursor: pointer;
}

.detail-section {
  display: grid;
  gap: 12px;
  margin-top: 24px;
}

.detail-section h3 {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  font-weight: 650;
}

.detail-section p {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-body);
  line-height: 1.85;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

@media (max-width: 600px) {

  .detail-hero {
    grid-template-columns: 88px minmax(0, 1fr);
    column-gap: 16px;
    row-gap: 12px;
  }

  .detail-hero>img, .detail-cover-fallback {
    width: 88px;
    height: 118px;
  }

  .prototype-overview-progress {
    margin-top: 20px;
  }

  .book-detail-facts {
    column-gap: 20px;
    margin-top: 8px;
  }

  .book-detail-fact {
    min-height: 72px;
  }
}
}
</style>
