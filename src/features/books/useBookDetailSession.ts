import { addLibraryContent } from "./contentLibraryState";
import { computed, reactive, ref, shallowRef, watch, type Ref, type UnwrapNestedRefs } from "vue";
import { cancelBookPreparation, prepareSearchResultBook } from "../../api/books";
import type { BookResource, SearchBookResult, SearchResultSource } from "../../api/types";
import { searchResults } from "../discovery/searchState";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import { clearOpenedBook, closeBookDetail, switchOpenedBook, openedBookFromShelf, openedBook } from "./bookDetailState";
import type { BookDetailOpenRequest, BookDetailTab } from "./bookDetailOpenRequest";

export interface BookDetailSessionState {
  result: SearchBookResult | null;
  bookId: string | null;
  inShelf: boolean;
  initialTab: BookDetailTab;
  error: string;
}
type ActiveBookDetailSession = UnwrapNestedRefs<BookDetailSessionState>;
let preparationRevision = Date.now();

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
  let generation = 0;
  let activePreparation: number | null = null;
  const sourceGroup = shallowRef<SearchBookResult | null>(null);
  const searchResultSources = computed(() => {
    const original = sourceGroup.value;
    if (!original) return [];
    const current = searchResults.value.find(group => group.resultId === original.resultId
      || group.sources?.some(source => source.resultId === original.resultId)) ?? original;
    return current.sources?.length ? current.sources : [current];
  });
  function cancelPreparation(): void {
    if (activePreparation === null) return;
    const revision = activePreparation;
    activePreparation = null;
    void cancelBookPreparation(revision).catch(error => console.error("Cancel book preparation failed", error));
  }
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
    cancelPreparation();
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
    const revision = preparationRevision = Math.max(preparationRevision + 1, Date.now());
    activePreparation = revision;
    try {
      const response = await prepareSearchResultBook(result.resultId, revision);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      const book = await switchOpenedBook(response.book);
      if (!book) return;
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      request.details.bookId = book.id;
    } catch (error) {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) {
        request.details.error = dependencies.errorText(error);
      }
    } finally {
      if (activePreparation === revision) activePreparation = null;
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
        throw new Error("\u4e66\u7c4d\u6587\u4ef6\u6682\u65f6\u4e0d\u53ef\u7528\u3002");
      }
      const book = await switchOpenedBook(bookId);
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
        dependencies.notify(`\u6253\u5f00\u9605\u8bfb\u5931\u8d25\uff1a${dependencies.errorText(error)}`, "error");
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
    cancelPreparation();
    sourceGroup.value = null;
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
