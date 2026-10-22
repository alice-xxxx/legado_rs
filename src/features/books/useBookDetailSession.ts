import { readBookAlternatives, readCachedMoreInformation, readMoreInformation } from "../../api/database";
import { modifyDatabase } from "../../api/feDb";
import { addLibraryContent } from "./contentLibraryState";
import { computed, reactive, ref, shallowRef, watch, type Ref, type UnwrapNestedRefs } from "vue";
import { moreInformation } from "../../api/moreInformation";
import type { BookResource, SearchBookResult, SearchResultSource } from "../../api/types";
import { searchResults } from "../discovery/searchState";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { clearOpenedBook, closeBookDetail, replaceOpenedBook, switchOpenedBook, openedBookFromShelf, openedBook } from "./bookDetailState";
import type { BookDetailOpenRequest, BookDetailTab } from "./bookDetailOpenRequest";

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
  closeDisplayEditor: () => void;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** Owns book-detail loading, reading, and add-to-shelf interactions for the detail page. */
export function useBookDetailSession(dependencies: BookDetailSessionDependencies) {
  const searchResultDetails = shallowRef<ActiveBookDetailSession | null>(null);
  const { bookPanelBusy } = dependencies;
  const searchResultDetailsReadingBusy = ref(false);
  let information: ReturnType<typeof moreInformation> | null = null;
  let generation = 0;
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
    if (bookPanelBusy.value) void information?.cancel().catch(console.error);
    generation += 1;
    const currentGeneration = generation;
    const restoreRevision = dependencies.getRestoreRevision();
    openedBookFromShelf.value = false;
    dependencies.closeDisplayEditor();
    searchResultDetailsReadingBusy.value = false;
    bookPanelBusy.value = true;
    clearOpenedBook();
    const details = reactive(state);
    searchResultDetails.value = details;
    return { generation: currentGeneration, restoreRevision, details };
  }

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
      let saved = await readCachedMoreInformation(result.sourceId, result.bookUrl);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      if (!saved) {
        information = moreInformation(result.sourceId, result.bookUrl);
        await information;
        if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
        saved = await readMoreInformation(result.sourceId, result.bookUrl);
      }
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      // 候选身份和关联在一个事务内保存，Vue 不接触数据表。
      const candidates = searchResultSources.value.filter(source => source.sourceId !== saved.sourceId);
      await modifyDatabase([
        { operation: "saveBooks", books: candidates.map(source => ({ sourceId: source.sourceId, bookUrl: source.bookUrl, title: source.title, author: source.author, cover: source.coverSrc, latest: source.latestChapter })) },
        { operation: "saveBook", bookId: saved.id, changes: { alternatives: candidates.map(source => source.resultId) } },
      ], { wait: true });
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      saved.alternatives = await readBookAlternatives(saved.id);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      replaceOpenedBook(saved);
      request.details.bookId = saved.id;
    } catch (error) {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) {
        request.details.error = dependencies.errorText(error);
      }
    } finally {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) bookPanelBusy.value = false;
    }
  }

  async function selectSearchResultSource(candidate: SearchResultSource): Promise<void> {
    if (dependencies.isRecoveryRequired() || candidate.resultId === searchResultDetails.value?.result?.resultId) return;
    await openSearchResultDetails(candidate, "sources", true);
  }

  async function openShelfBook(bookId: string, initialTab: BookDetailTab = "catalog", fromShelf = true): Promise<void> {
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
      let book = await switchOpenedBook(bookId);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      if (book?.sourceId && book.bookUrl && book.updatedAtMs == null && !book.chapters.length) {
        information = moreInformation(book.sourceId, book.bookUrl);
        await information;
        if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
        const saved = await readMoreInformation(book.sourceId, book.bookUrl);
        if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
        replaceOpenedBook(saved);
        book = saved;
      }
      if (!book) return;
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
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

  async function addSearchResultFromDetails(): Promise<void> {
    const book = openedBook.value;
    if (!book || searchResultDetailsShelfBook.value) return;
    addLibraryContent(book);
    if (searchResultDetails.value) searchResultDetails.value.inShelf = true;
  }

  function clear(options: { preserveOpenedBook?: boolean } = {}): void {
    sourceGroup.value = null;
    void information?.cancel().catch(console.error);
    generation += 1;
    bookPanelBusy.value = false;
    searchResultDetailsReadingBusy.value = false;
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
