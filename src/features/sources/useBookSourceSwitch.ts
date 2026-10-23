import { replaceLibraryContent, libraryType, refreshContentLibrary } from "../books/contentLibraryState";
import { computed, ref, shallowRef, type Ref } from "vue";
import { bookSourceCandidates } from "./sourcesStore";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { search, stopSearch, releaseSearchResults } from "../../api/search";
import { mainText, cancelMainText } from "../../api/mainText";
import { saveChapter } from "../../api/books";
import { modifyDatabase } from "../../api/feDb";
import { moreInformation } from "../../api/moreInformation";
import { readChapter, readMoreInformation } from "../../api/database";
import { bookContentType } from "../../api/bookContentType";
import { compareSearchSources, presentSearchResults } from "../discovery/searchPresentation";
import { type BookResource, type SearchBookResult } from "../../api/types";
import type { CachedChapterContent } from "../reader/useReaderChapterLoading";
import { refreshOpenedBook, refreshOpenedChapter, replaceOpenedBook, openedBook } from "../books/bookDetailState";

interface BookSourceSwitchDependencies {
  /** 应用共享的任务、书架、阅读器和书签状态。 */
  readingBook: Ref<BookResource | null>;
  readerVisible: Ref<boolean>;
  readingChapterIndex: Ref<number>;
  readerPageIndex: Ref<number>;
  readingContent: Ref<CachedChapterContent | null>;
  readerProgressDirty: Ref<boolean>;
  readerControlsOpen: Ref<boolean>;
  saveCurrentProgress: () => Promise<void>;
  loadReaderChapter: (index: number) => Promise<boolean>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  getRestoreRevision: () => number;
}

/** 管理整书换源、单章正文替换和搜索通知。 */
export function useBookSourceSwitch({
  readingBook,
  readerVisible,
  readingChapterIndex,
  readerPageIndex,
  readingContent,
  readerProgressDirty,
  readerControlsOpen,
  saveCurrentProgress,
  loadReaderChapter,
  notify,
  errorText,
  getRestoreRevision,
}: BookSourceSwitchDependencies) {
  const bookSourceSwitchOpen = ref(false);
  const bookSourceSwitchInline = ref(false);
  const bookSourceSwitchBusy = ref(false);
  const bookSourceSwitchCancellable = ref(false);
  const bookSourceCheckBusy = ref(false);
  const chapterSourceBusyResultId = ref<string | null>(null);
  const chapterSourceError = ref("");
  const bookSourceSearchBusy = ref(false);
  const candidateRetryBusy = ref(false);
  const bookSourceSwitchConfirmOpen = ref(false);
  const bookSourceSwitchFromReader = ref(false);
  const bookSourceSwitchBookId = ref<string | null>(null);
  const bookSourceSwitchBook = computed(() => !bookSourceSwitchOpen.value
    || openedBook.value?.id === bookSourceSwitchBookId.value ? openedBook.value : null);
  const bookSourceEligibleSources = computed(() => {
    const book = bookSourceSwitchBook.value;
    if (!book || book.canChangeSource !== true) return [];
    const type = bookContentType(book);
    const searchType = type === "html" || type === "pdf" ? "text" : type
      ?? bookSourceCandidates.value.find(source => source.id === book.sourceId)?.searchType ?? "text";
    return bookSourceCandidates.value.filter(source => source.id !== book.sourceId && source.searchType === searchType);
  });
  const bookSourceChapterTargetId = ref<string | null>(null);
  const bookSourceChapterTargetTitle = ref<string | null>(null);
  const bookSourceSearchId = ref<string | null>(null);
  const searchedCandidates = shallowRef<SearchBookResult[]>([]);
  const bookSourceCandidateResults = computed<SearchBookResult[]>(() => {
    const cached = (bookSourceSwitchBook.value?.alternatives ?? []).map(candidate => ({
      resultId: candidate.resultId,
      sourceId: candidate.sourceId, sourceName: candidate.sourceName, bookUrl: candidate.bookUrl,
      title: candidate.title, author: candidate.author, coverSrc: candidate.cover,
      latestChapter: candidate.latest,
      intro: candidate.intro, responseTimeMs: candidate.responseTimeMs,
    }));
    if (!bookSourceSwitchOpen.value) return cached.sort(compareSearchSources);
    const liveSources = new Set(searchedCandidates.value.map(candidate => candidate.sourceId));
    return [...cached.filter(candidate => !liveSources.has(candidate.sourceId)), ...searchedCandidates.value].sort(compareSearchSources);
  });
  const bookSourceCandidateErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
  const selectedBookSourceCandidate = ref<SearchBookResult | null>(null);
  const bookSourceSwitchError = ref("");
  const bookSourceChangeError = ref("");
  const bookSourceSearchStatus = ref("");
  const bookSourceSearchKeyword = ref("");
    let bookSourceSearchGeneration = 0;
  let information: ReturnType<typeof moreInformation> | null = null;
  let text: ReturnType<typeof mainText> | null = null;
  let cancellation: Promise<void> = Promise.resolve();
  const currentRestore = (revision: number) => revision === getRestoreRevision();

  const bookSourceCurrentChapter = computed(() => {
    const book = bookSourceSwitchBook.value;
    if (!book) return null;
    const currentBook = openedBook.value?.id === book.id
      ? openedBook.value
      : readingBook.value?.id === book.id ? readingBook.value : book;
    if (bookSourceChapterTargetId.value) {
      return currentBook.chapters.find((chapter) => chapter.id === bookSourceChapterTargetId.value) ?? null;
    }
    const readerBook = readingBook.value;
    if (bookSourceSwitchFromReader.value && readerVisible.value && readerBook?.id === book.id) {
      return readerBook.chapters.find((chapter) => chapter.index === readingChapterIndex.value) ?? null;
    }
    const chapterIndex = book.progress?.chapterIndex;
    if (chapterIndex == null) return null;
    return book.chapters.find((chapter) => chapter.index === chapterIndex) ?? null;
  });

  async function openBookSourceSwitch(fromReader = false, chapterTarget?: { id: string; title: string }): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const book = fromReader ? readingBook.value : openedBook.value;
    await openBookSourceSwitchFor(book, fromReader, chapterTarget, restoreRevision);
  }

  async function openBookSourceSwitchForBook(book: BookResource, chapterTarget?: { id: string; title: string }, inline = false): Promise<void> {
    await openBookSourceSwitchFor(book, false, chapterTarget, getRestoreRevision(), inline);
  }

  async function openBookSourceSwitchFor(
    book: BookResource | null,
    fromReader: boolean,
    chapterTarget: { id: string; title: string } | undefined,
    restoreRevision: number,
    inline = false,
  ): Promise<void> {
    if (!book || book.canChangeSource !== true || bookSourceSwitchBusy.value || bookSourceCheckBusy.value || candidateRetryBusy.value) return;
    if (bookSourceSwitchOpen.value && bookSourceSwitchBookId.value === book.id
      && bookSourceSwitchFromReader.value === fromReader && bookSourceChapterTargetId.value === (chapterTarget?.id ?? null)) {
      bookSourceSwitchInline.value = inline;
      return;
    }
    if (fromReader) {
      readerControlsOpen.value = false;
      await saveCurrentProgress();
      if (!currentRestore(restoreRevision)) return;
      if (readerProgressDirty.value) {
        notify("阅读位置尚未保存，请稍后再试。", "error");
        return;
      }
    }
    closeBookSourceSwitch();
    bookSourceSearchGeneration += 1;
    bookSourceSwitchBookId.value = book.id;
    bookSourceSwitchFromReader.value = fromReader;
    bookSourceChapterTargetId.value = chapterTarget?.id ?? null;
    bookSourceChapterTargetTitle.value = chapterTarget?.title ?? null;
    bookSourceSearchId.value = null;
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    chapterSourceError.value = "";
    bookSourceSwitchError.value = "";
    bookSourceChangeError.value = "";
    bookSourceSearchStatus.value = "";
    bookSourceSearchKeyword.value = "";
    bookSourceSearchBusy.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceSwitchOpen.value = true;
    bookSourceSwitchInline.value = inline;
    bookSourceSearchStatus.value = bookSourceCandidateResults.value.length ? "已缓存的可换来源" : "尚无可换来源，可点击「搜索更多」查找。";
  }

  function closeBookSourceSwitch(): void {
    cancellation = Promise.allSettled([cancellation, information?.cancel(), text?.cancel()]).then(() => {});
    information = null;
    text = null;
    bookSourceSwitchBusy.value = false;
    bookSourceSwitchCancellable.value = false;
    chapterSourceBusyResultId.value = null;
    const taskId = bookSourceSearchId.value;
    const shouldCancel = Boolean(taskId);
    bookSourceSearchGeneration += 1;
    bookSourceSearchId.value = null;
    bookSourceSwitchOpen.value = false;
    bookSourceCheckBusy.value = false;
    bookSourceSwitchInline.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceChapterTargetId.value = null;
    bookSourceChapterTargetTitle.value = null;
    selectedBookSourceCandidate.value = null;
    chapterSourceError.value = "";
    bookSourceSearchBusy.value = false;
    bookSourceChangeError.value = "";
    if (shouldCancel && taskId) void releaseSearchResults(taskId).catch(() => {});
  }

  async function startBookSourceCandidateSearch(): Promise<void> {
    if (shelfBatchRecoveryRequired.value) return;
    const book = bookSourceSwitchBook.value;
    if (!book || book.canChangeSource !== true || bookSourceSearchBusy.value || bookSourceSwitchBusy.value || bookSourceCheckBusy.value) return;
    const restoreRevision = getRestoreRevision();
    const generation = ++bookSourceSearchGeneration;
    const sourceIds = bookSourceEligibleSources.value.map((source) => source.id);
    if (!sourceIds.length) {
      bookSourceSwitchError.value = "没有可用于换源的已启用源。";
      return;
    }
    bookSourceSwitchError.value = "";
    chapterSourceError.value = "";
    if (bookSourceSearchId.value) void releaseSearchResults(bookSourceSearchId.value).catch(() => {});
    bookSourceSearchId.value = null;
    bookSourceSearchStatus.value = "正在搜索其他源…";
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceSearchBusy.value = true;
    let candidates: SearchBookResult[] = [];
    try {
      const manualKeyword = bookSourceSearchKeyword.value.trim();
      // 换源与其他页面共用搜索接口，只在此选择 100% 匹配。
      const keyword = manualKeyword || book.title;
      const results: SearchBookResult[] = [];
      let completed = 0;
      const id = await search(sourceIds, keyword, 1, "exact", batch => {
        if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) {
          void releaseSearchResults(batch.searchId).catch(console.error);
          return;
        }
        bookSourceSearchId.value = batch.searchId;
        results.push(...batch.results);
        searchedCandidates.value = presentSearchResults(results, keyword)
          .flatMap(group => group.sources!.map(source => ({ ...source, sources: group.sources })));
        if (batch.results.length) candidates = bookSourceCandidateResults.value.filter(source => source.sourceId !== book.sourceId);
        if (batch.error) bookSourceCandidateErrors.value.push({ sourceId: batch.sourceId, message: batch.error });
        if (batch.sourceId) completed++;
        bookSourceSearchStatus.value = `正在搜索其他源（${completed} / ${sourceIds.length}）`;
      });
      if (!currentRestore(restoreRevision)) return;
      if (generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) {
        await releaseSearchResults(id);
      } else {
        bookSourceSearchBusy.value = false;
        bookSourceSearchStatus.value = `找到 ${bookSourceCandidateResults.value.length} 个可用源`;
      }
    } catch (error) {
      if (currentRestore(restoreRevision) && generation === bookSourceSearchGeneration && bookSourceSwitchOpen.value) {
        bookSourceSearchBusy.value = false;
        bookSourceSwitchError.value = errorText(error);
        bookSourceSearchStatus.value = "搜索失败";
      }
    } finally {
      // 正常结束或取消后统一保存一次；未上架的书仍由加入书架操作保存。
      if (candidates.length && currentRestore(restoreRevision) && libraryType(book.id)) {
        book.alternatives = candidates.map(source => ({ ...source, cover: source.coverSrc, latest: source.latestChapter }));
        await modifyDatabase([
          { operation: "saveBooks", books: candidates.map(source => ({ sourceId: source.sourceId, bookUrl: source.bookUrl, title: source.title, author: source.author, cover: source.coverSrc, latest: source.latestChapter })) },
          { operation: "saveBook", bookId: book.id, changes: { alternatives: candidates.map(source => source.resultId) } },
        ], { wait: true }).catch(error => {
          if (currentRestore(restoreRevision)) notify(`保存可换来源失败：${errorText(error)}`, "error");
        });
      }
    }
  }

  async function stopBookSourceCandidateSearch(): Promise<void> {
    const taskId = bookSourceSearchId.value;
    if (taskId) {
      try { await stopSearch(taskId); }
      catch (error) { bookSourceSwitchError.value = errorText(error); }
    } else {
      // Admission may still be pending; its eventual response will cancel itself.
      bookSourceSearchGeneration += 1;
      bookSourceSearchBusy.value = false;
      bookSourceSearchStatus.value = "已停止搜索";
    }
  }

  function selectBookSourceCandidate(candidate: SearchBookResult): void {
    selectedBookSourceCandidate.value = candidate;
    bookSourceChangeError.value = "";
    chapterSourceError.value = "";
  }

  async function chooseBookSourceCandidate(candidate: SearchBookResult): Promise<void> {
    if (shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value || bookSourceCheckBusy.value) return;
    selectBookSourceCandidate(candidate);
    await confirmBookSourceChange();
  }

  async function checkBookSourceUpdates(): Promise<void> {
    const book = bookSourceSwitchBook.value;
    if (!book?.sourceId || !book.bookUrl || shelfBatchRecoveryRequired.value
      || bookSourceCheckBusy.value || bookSourceSwitchBusy.value) return;
    const revision = getRestoreRevision(), generation = bookSourceSearchGeneration;
    const current = () => currentRestore(revision) && generation === bookSourceSearchGeneration
      && bookSourceSwitchOpen.value && openedBook.value?.id === book.id;
    const previous = new Set(book.chapters.map(chapter => chapter.id));
    bookSourceCheckBusy.value = true;
    bookSourceSwitchError.value = "";
    try {
      await cancellation;
      if (!current()) return;
      information = moreInformation(book.sourceId, book.bookUrl);
      await information;
      if (!current()) return;
      const updated = await refreshOpenedBook(book.id);
      if (!current() || !updated) return;
      if (readerVisible.value && readingBook.value?.id === book.id) {
        readingChapterIndex.value = updated.progress?.chapterIndex ?? readingChapterIndex.value;
      }
      if (libraryType(book.id)) await refreshContentLibrary();
      if (!current()) return;
      const added = updated.chapters.filter(chapter => !chapter.volume && !previous.has(chapter.id)).length;
      notify(added ? `发现 ${added} 个新章节，目录已更新。` : "暂无新章节。");
    } catch (error) {
      if (current()) bookSourceSwitchError.value = `检查内容更新失败：${errorText(error)}`;
    } finally {
      if (currentRestore(revision) && generation === bookSourceSearchGeneration) bookSourceCheckBusy.value = false;
    }
  }

  async function replaceCurrentChapterFromCandidate(candidate: SearchBookResult): Promise<void> {
    const book = bookSourceSwitchBook.value, chapter = bookSourceCurrentChapter.value;
    if (!book || !chapter || shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value || bookSourceCheckBusy.value) return;
    const revision = getRestoreRevision(), generation = bookSourceSearchGeneration;
    const current = () => currentRestore(revision) && generation === bookSourceSearchGeneration && bookSourceSwitchOpen.value;
    chapterSourceBusyResultId.value = candidate.resultId;
    chapterSourceError.value = "";
    bookSourceSwitchBusy.value = true;
    bookSourceSwitchCancellable.value = true;
    try {
      await cancellation;
      if (!current()) return;
      information = moreInformation(candidate.sourceId, candidate.bookUrl);
      await information;
      if (!current()) return;
      const source = await readMoreInformation(candidate.sourceId, candidate.bookUrl);
      if (!current()) return;
      const normalize = (title: string) => title.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
      const matches = source.chapters.filter(item => !item.volume && normalize(item.title) === normalize(chapter.title));
      if (matches.length !== 1) throw new Error(matches.length ? "有多个同名章节，请选择唯一章节。" : "该源没有对应章节。");
      // 正文只写入候选书籍；缓存完成后，Vue 修改原书的章节地址。
      text = mainText(source.id, matches[0]!.id);
      await text;
      if (!current()) return;
      const replacement = await readChapter(source, matches[0]!.id);
      if (!current()) return;
      if (!replacement?.chapter.contentId) throw new Error("候选章节正文尚未缓存。");
      await cancelMainText(book.id, chapter.id);
      if (!current()) return;
      bookSourceSwitchCancellable.value = false;
      await saveChapter(book.id, { ...chapter, contentId: replacement.chapter.contentId });
      if (!current()) return;
      await refreshOpenedChapter(book.id, chapter.id);
      if (!current()) return;
      if (readingBook.value?.id === book.id) {
        if (readerVisible.value && readingChapterIndex.value === chapter.index) {
          readingContent.value = null;
          await loadReaderChapter(chapter.index);
        }
      }
      if (!current()) return;
      notify(`《${book.title}》的章节“${chapter.title}”正文已替换。`);
      closeBookSourceSwitch();
    } catch (error) {
      if (!current()) return;
      chapterSourceError.value = errorText(error);
      notify(chapterSourceError.value, "error");
    } finally {
      if (currentRestore(revision) && generation === bookSourceSearchGeneration) {
        chapterSourceBusyResultId.value = null;
        bookSourceSwitchBusy.value = false;
        bookSourceSwitchCancellable.value = false;
      }
    }
  }

  async function confirmBookSourceChange(): Promise<void> {
    const book = bookSourceSwitchBook.value, candidate = selectedBookSourceCandidate.value;
    if (!book || !candidate || shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value || bookSourceCheckBusy.value) return;
    const revision = getRestoreRevision(), generation = bookSourceSearchGeneration;
    const current = () => currentRestore(revision) && generation === bookSourceSearchGeneration && bookSourceSwitchOpen.value;
    const readerWasOpen = bookSourceSwitchFromReader.value && readerVisible.value && readingBook.value?.id === book.id;
    bookSourceSwitchBusy.value = true;
    bookSourceSwitchCancellable.value = true;
    bookSourceChangeError.value = "";
    try {
      await cancellation;
      if (!current()) return;
      if (readerWasOpen) {
        await saveCurrentProgress();
        if (!current()) return;
        if (readerProgressDirty.value) throw new Error("阅读位置尚未保存，请稍后再试。");
      }
      information = moreInformation(candidate.sourceId, candidate.bookUrl);
      await information;
      if (!current()) return;
      const cached = await readMoreInformation(candidate.sourceId, candidate.bookUrl);
      if (!current()) return;
      const alternatives = [{ resultId: book.id, sourceId: book.sourceId!, sourceName: book.sourceName ?? "", bookUrl: book.bookUrl!,
        title: book.title, author: book.author, coverSrc: book.coverSrc, latestChapter: book.latestChapter }, ...bookSourceCandidateResults.value]
        .filter(source => source.sourceId !== cached.sourceId);
      // 只在上架的新来源上保存关联；未上架时只保留内存中的来源列表。
      if (libraryType(book.id)) {
        await modifyDatabase([
          { operation: "saveBooks", books: alternatives.map(source => ({ sourceId: source.sourceId, bookUrl: source.bookUrl, title: source.title, author: source.author, cover: source.coverSrc, latest: source.latestChapter })) },
          { operation: "saveBook", bookId: cached.id, changes: { alternatives: alternatives.map(source => source.resultId) } },
        ], { wait: true });
        if (!current()) return;
      }
      cached.alternatives = alternatives.map(source => ({ ...source, cover: source.coverSrc, latest: source.latestChapter }));
      bookSourceSwitchCancellable.value = false;
      // 整书换源只替换书架条目，书架现有监听负责持久化；两本书的缓存各自保留。
      if (libraryType(book.id)) replaceLibraryContent(book.id, cached);
      if (openedBook.value?.id === book.id) replaceOpenedBook(cached);
      if (!current()) return;
      closeBookSourceSwitch();
      notify(`已切换到“${candidate.sourceName}”。`);
      if (readerWasOpen) {
        readingBook.value = cached;
        readingChapterIndex.value = cached.progress?.chapterIndex ?? 0;
        readerPageIndex.value = cached.progress?.offset ?? 0;
        readingContent.value = null;
        await loadReaderChapter(readingChapterIndex.value);
      }
    } catch (error) {
      if (current()) bookSourceChangeError.value = errorText(error);
    } finally {
      if (currentRestore(revision) && generation === bookSourceSearchGeneration) {
        bookSourceSwitchBusy.value = false;
        bookSourceSwitchCancellable.value = false;
      }
    }
  }
  function resetForRestore(): void {
    // Reset without calling closeBookSourceSwitch: cancelling by an ID from
    // the old library could cancel an unrelated restored task with that ID.
    bookSourceSearchGeneration += 1;
    bookSourceSwitchOpen.value = false;
    bookSourceSwitchInline.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceSwitchBusy.value = false;
    bookSourceSwitchCancellable.value = false;
    cancellation = Promise.resolve();
    information = null;
    text = null;
    bookSourceCheckBusy.value = false;
    chapterSourceBusyResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSearchBusy.value = false;
    candidateRetryBusy.value = false;
    bookSourceSwitchFromReader.value = false;
    bookSourceSwitchBookId.value = null;
    bookSourceChapterTargetId.value = null;
    bookSourceChapterTargetTitle.value = null;
    bookSourceSearchId.value = null;
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceSwitchError.value = "";
    bookSourceChangeError.value = "";
    bookSourceSearchStatus.value = "";
    bookSourceSearchKeyword.value = "";
  }

  return {
    resetForRestore,
    bookSourceSwitchOpen,
    bookSourceSwitchInline,
    bookSourceSwitchBusy,
    bookSourceSwitchCancellable,
    bookSourceCheckBusy,
    chapterSourceBusyResultId,
    chapterSourceError,
    bookSourceSearchBusy,
    bookSourceSwitchConfirmOpen,
    bookSourceSwitchBook,
    bookSourceEligibleSources,
    bookSourceChapterTargetId,
    bookSourceChapterTargetTitle,
    bookSourceSearchId,
    bookSourceCandidateResults,
    bookSourceCandidateErrors: computed(() => bookSourceSwitchBookId.value === openedBook.value?.id
      ? bookSourceCandidateErrors.value : []),
    selectedBookSourceCandidate,
    bookSourceSwitchError,
    bookSourceChangeError,
    bookSourceSearchStatus,
    bookSourceSearchKeyword,
    bookSourceCurrentChapter,
    openBookSourceSwitch,
    openBookSourceSwitchForBook,
    closeBookSourceSwitch,
    startBookSourceCandidateSearch,
    stopBookSourceCandidateSearch,
    selectBookSourceCandidate,
    chooseBookSourceCandidate,
    checkBookSourceUpdates,
    replaceCurrentChapterFromCandidate,
    confirmBookSourceChange,
  };
}
