import { readBookAlternatives, readCachedMoreInformation, readMoreInformation } from "../../api/database";
import { modifyDatabase } from "../../api/feDb";
import { addLibraryContent, libraryType, refreshContentLibrary } from "./contentLibraryState";
import { computed, reactive, ref, shallowRef, watch, type Ref, type UnwrapNestedRefs } from "vue";
import { moreInformation } from "../../api/moreInformation";
import type { BookResource, SearchBookResult, SearchResultSource } from "../../api/types";
import { searchResults } from "../discovery/searchState";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { clearOpenedBook, closeBookDetail, replaceOpenedBook, switchOpenedBook, openedBookFromShelf, openedBook } from "./bookDetailState";
import type { BookDetailOpenRequest, BookDetailTab } from "./bookDetailOpenRequest";
import { hasBookInformation, shouldRefreshBookInformation } from "./bookInformationPolicy";

export interface BookDetailSessionState {
  result: SearchBookResult | null;
  bookId: string | null;
  inShelf: boolean;
  initialTab: BookDetailTab;
  error: string;
}
type ActiveBookDetailSession = UnwrapNestedRefs<BookDetailSessionState>;

export interface BookDetailSessionDependencies {
  /** Shared busy state lets other book actions keep their existing exclusion guards. */
  bookPanelBusy: Ref<boolean>;
  isRecoveryRequired: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, chapterIndex?: number) => Promise<void>;
  applyBookInformation?: (book: BookResource) => void;
  closeDisplayEditor: () => void;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** Owns book-detail loading, reading, and add-to-shelf interactions for the detail page. */
export function useBookDetailSession(dependencies: BookDetailSessionDependencies) {
  const searchResultDetails = shallowRef<ActiveBookDetailSession | null>(null);
  const { bookPanelBusy } = dependencies;
  const searchResultDetailsReadingBusy = ref(false);
  const informationRefreshBusy = ref(false);
  const informationRefreshError = ref("");
  let information: ReturnType<typeof moreInformation> | null = null;
  let informationCancellation: Promise<void> = Promise.resolve();
  let generation = 0;
  let addingToShelf = false;
  const sourceGroup = shallowRef<SearchBookResult | null>(null);
  const searchResultSources = computed(() => {
    const original = sourceGroup.value;
    if (!original) return [];
    const current = searchResults.value.find(group => group.resultId === original.resultId
      || group.sources?.some(source => source.resultId === original.resultId)) ?? original;
    return current.sources?.length ? current.sources : [current];
  });
  const searchResultDetailsShelfBook = computed(() => {
    const bookId = openedBook.value?.id;
    return bookId ? shelf.value.books.find((book) => book.id === bookId) ?? null : null;
  });
  watch(searchResultDetailsShelfBook, (book) => {
    if (searchResultDetails.value) searchResultDetails.value.inShelf = Boolean(book);
  });

  watch(() => openedBook.value?.id, (id) => {
    if (id && searchResultDetails.value) searchResultDetails.value.bookId = id;
  });

  function isCurrentRequest(requestGeneration: number, restoreRevision: number, details: ActiveBookDetailSession): boolean {
    return requestGeneration === generation
      && restoreRevision === dependencies.getRestoreRevision()
      && searchResultDetails.value === details;
  }

  function beginSession(state: BookDetailSessionState): {
    generation: number;
    restoreRevision: number;
    details: ActiveBookDetailSession;
  } {
    informationCancellation = information?.cancel().catch(console.error) ?? Promise.resolve();
    generation += 1;
    const currentGeneration = generation;
    const restoreRevision = dependencies.getRestoreRevision();
    openedBookFromShelf.value = false;
    dependencies.closeDisplayEditor();
    searchResultDetailsReadingBusy.value = false;
    informationRefreshBusy.value = false;
    informationRefreshError.value = "";
    bookPanelBusy.value = true;
    clearOpenedBook();
    const details = reactive(state);
    searchResultDetails.value = details;
    return { generation: currentGeneration, restoreRevision, details };
  }

  type DetailRequest = ReturnType<typeof beginSession>;
  const requestIsCurrent = (request: DetailRequest) => isCurrentRequest(request.generation, request.restoreRevision, request.details);

  async function fetchInformation(sourceId: string, bookUrl: string, request: DetailRequest): Promise<BookResource | null> {
    // 同一源和 URL 的旧请求取消完成后再启动，避免取消信号误伤新请求。
    await informationCancellation;
    if (!requestIsCurrent(request)) return null;
    information = moreInformation(sourceId, bookUrl);
    await information;
    if (!requestIsCurrent(request)) return null;
    const book = await readMoreInformation(sourceId, bookUrl);
    return requestIsCurrent(request) ? book : null;
  }

  async function resolveInformation(cached: BookResource | null, sourceId: string | undefined, bookUrl: string | undefined, request: DetailRequest) {
    if (cached && hasBookInformation(cached)) return { book: cached, cached: true };
    if (!sourceId || !bookUrl) throw new Error("书籍详情暂时不可用。");
    const book = await fetchInformation(sourceId, bookUrl, request);
    return book ? { book, cached: false } : null;
  }

  async function refreshInformationInBackground(book: BookResource, request: DetailRequest): Promise<void> {
    if (!book.sourceId || !book.bookUrl || informationRefreshBusy.value || !requestIsCurrent(request)) return;
    const current = () => requestIsCurrent(request) && openedBook.value?.id === book.id;
    informationRefreshBusy.value = true;
    informationRefreshError.value = "";
    try {
      const updated = await fetchInformation(book.sourceId, book.bookUrl, request);
      if (!updated || !current()) return;
      (dependencies.applyBookInformation ?? replaceOpenedBook)(updated);
      if (libraryType(updated.id)) await refreshContentLibrary();
    } catch (error) {
      if (current()) informationRefreshError.value = `刷新详情失败，正在显示缓存：${dependencies.errorText(error)}`;
    } finally {
      if (requestIsCurrent(request)) informationRefreshBusy.value = false;
    }
  }

  function displayInformation(book: BookResource, request: DetailRequest): void {
    replaceOpenedBook(book);
    request.details.bookId = book.id;
    bookPanelBusy.value = false;
  }

  watch(() => openedBook.value?.informationFetchedAtMs, () => { informationRefreshError.value = ""; });

  async function openSearchResultDetails(
    result: SearchBookResult,
    initialTab: BookDetailTab = "info",
    preserveSourceGroup = false,
  ): Promise<void> {
    if (!preserveSourceGroup) sourceGroup.value = result;
    const request = beginSession({
      result,
      bookId: null,
      inShelf: false,
      initialTab,
      error: "",
    });
    try {
      const cached = await readCachedMoreInformation(result.sourceId, result.bookUrl);
      if (!requestIsCurrent(request)) return;
      const resolved = await resolveInformation(cached, result.sourceId, result.bookUrl, request);
      if (!resolved || !requestIsCurrent(request)) return;
      const saved = resolved.book;
      displayInformation(saved, request);
      if (resolved.cached && shouldRefreshBookInformation(saved, result.latestChapter)) {
        void refreshInformationInBackground(saved, request);
      }
    } catch (error) {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) {
        if (openedBook.value) informationRefreshError.value = dependencies.errorText(error);
        else request.details.error = dependencies.errorText(error);
      }
    } finally {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) bookPanelBusy.value = false;
    }
  }

  async function selectSearchResultSource(candidate: SearchResultSource): Promise<void> {
    if (dependencies.isRecoveryRequired() || candidate.resultId === searchResultDetails.value?.result?.resultId) return;
    await openSearchResultDetails(candidate, "sources", true);
  }

  async function openShelfBook(bookId: string, initialTab: BookDetailTab = "info", fromShelf = true): Promise<void> {
    sourceGroup.value = null;
    const request = beginSession({
      result: null,
      bookId,
      inShelf: shelf.value.books.some((entry) => entry.id === bookId),
      initialTab,
      error: "",
    });
    openedBookFromShelf.value = fromShelf;
    try {
      if (fromShelf && !shelf.value.books.some((entry) => entry.id === bookId)) {
        throw new Error("书籍文件暂时不可用。");
      }
      const book = await switchOpenedBook(bookId);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      if (!book) return;
      const resolved = await resolveInformation(book, book.sourceId, book.bookUrl, request);
      if (!resolved || !requestIsCurrent(request)) return;
      if (resolved.book !== book) displayInformation(resolved.book, request);
      else bookPanelBusy.value = false;
      const latest = searchResults.value.flatMap(group => group.sources?.length ? group.sources : [group])
        .find(result => result.sourceId === book.sourceId && result.bookUrl === book.bookUrl)?.latestChapter;
      if (resolved.cached && shouldRefreshBookInformation(resolved.book, latest)) {
        void refreshInformationInBackground(resolved.book, request);
      }
    } catch (error) {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) {
        request.details.error = dependencies.errorText(error);
      }
    } finally {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) bookPanelBusy.value = false;
    }
  }

  async function openRequest(request: BookDetailOpenRequest): Promise<void> {
    if (request.kind === "search-result") {
      await openSearchResultDetails(request.result);
      return;
    }
    await openShelfBook(request.bookId, request.initialTab, request.kind === "shelf-book");
  }

  async function retry(): Promise<void> {
    const details = searchResultDetails.value;
    if (!details) return;
    if (details.result) {
      await openSearchResultDetails(details.result, details.initialTab, true);
      return;
    }
    if (details.bookId) await openShelfBook(details.bookId, details.initialTab, openedBookFromShelf.value);
  }

  async function startReading(chapterIndex?: number): Promise<void> {
    const details = searchResultDetails.value;
    const book = openedBook.value;
    if (!details || !book || bookPanelBusy.value || searchResultDetailsReadingBusy.value
      || dependencies.isRecoveryRequired()) return;
    const requestGeneration = generation;
    const restoreRevision = dependencies.getRestoreRevision();
    searchResultDetailsReadingBusy.value = true;
    try {
      if (!isCurrentRequest(requestGeneration, restoreRevision, details)) return;
      await dependencies.startReading(book, chapterIndex);
    } catch (error) {
      if (isCurrentRequest(requestGeneration, restoreRevision, details)) {
        dependencies.notify(`打开阅读失败：${dependencies.errorText(error)}`, "error");
      }
    } finally {
      if (isCurrentRequest(requestGeneration, restoreRevision, details)) searchResultDetailsReadingBusy.value = false;
    }
  }

  async function addSearchResultFromDetails(sources: SearchResultSource[] = searchResultSources.value): Promise<void> {
    const book = openedBook.value;
    const details = searchResultDetails.value;
    if (!book || !details || searchResultDetailsShelfBook.value || addingToShelf || dependencies.isRecoveryRequired()) return;
    const request = { generation, restoreRevision: dependencies.getRestoreRevision(), details };
    addingToShelf = true;
    try {
      const cached = (book.alternatives ?? []).map(source => ({ ...source, coverSrc: source.cover, latestChapter: source.latest }));
      const candidates = [...new Map([...cached, ...sources]
        .filter(source => source.sourceId !== book.sourceId).map(source => [source.resultId, source])).values()];
      if (candidates.length) {
        await modifyDatabase([
          { operation: "saveBooks", books: candidates.map(source => ({ sourceId: source.sourceId, bookUrl: source.bookUrl, title: source.title, author: source.author, cover: source.coverSrc, latest: source.latestChapter })) },
          { operation: "saveBook", bookId: book.id, changes: { alternatives: candidates.map(source => source.resultId) } },
        ], { wait: true });
      }
      if (!requestIsCurrent(request) || openedBook.value?.id !== book.id) return;
      if (candidates.length) openedBook.value.alternatives = await readBookAlternatives(book.id);
      if (!requestIsCurrent(request) || openedBook.value?.id !== book.id) return;
      addLibraryContent(openedBook.value);
      details.inShelf = true;
    } catch (error) {
      if (requestIsCurrent(request)) dependencies.notify(`加入书架失败：${dependencies.errorText(error)}`, "error");
    } finally { addingToShelf = false; }
  }

  function clear(options: { preserveOpenedBook?: boolean } = {}): void {
    sourceGroup.value = null;
    void information?.cancel().catch(console.error);
    generation += 1;
    bookPanelBusy.value = false;
    searchResultDetailsReadingBusy.value = false;
    informationRefreshBusy.value = false;
    informationRefreshError.value = "";
    searchResultDetails.value = null;
    closeBookDetail(Boolean(options.preserveOpenedBook));
    dependencies.closeDisplayEditor();
  }

  return {
    searchResultDetails,
    searchResultSources,
    selectSearchResultSource,
    searchResultDetailsShelfBook,
    searchResultDetailsReadingBusy,
    informationRefreshBusy,
    informationRefreshError,
    bookPanelBusy,
    openRequest,
    openSearchResultDetails,
    openShelfBook,
    retry,
    startReading,
    addSearchResultFromDetails,
    clear,
  };
}
