import { computed, reactive, ref, shallowRef, watch, type Ref, type UnwrapNestedRefs } from "vue";
import { getBook, prepareSearchResultBook } from "../../api/books";
import { readResource } from "../../api/resources";
import type { BookResource, SearchBookResult, ShelfResource } from "../../api/types";
import type { BookDetailOpenRequest, BookDetailSearchResultActions, BookDetailTab } from "./bookDetailOpenRequest";

export interface BookDetailSessionState {
  result: SearchBookResult | null;
  bookId: string | null;
  inShelf: boolean;
  initialTab: BookDetailTab;
  error: string;
}
type ActiveBookDetailSession = UnwrapNestedRefs<BookDetailSessionState>;

export interface BookDetailSessionDependencies {
  openedBook: Ref<BookResource | null>;
  shelf: Ref<ShelfResource>;
  /** Shared busy state lets other book actions keep their existing exclusion guards. */
  bookPanelBusy: Ref<boolean>;
  isRecoveryRequired: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, chapterIndex?: number) => Promise<void>;
  closeDisplayEditor: () => void;
  refreshShelf: () => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** Owns book-detail loading, reading, and add-to-shelf interactions for the detail page. */
export function useBookDetailSession(dependencies: BookDetailSessionDependencies) {
  const searchResultDetails = shallowRef<ActiveBookDetailSession | null>(null);
  const { bookPanelBusy } = dependencies;
  const searchResultDetailsReadingBusy = ref(false);
  let generation = 0;
  const activeSearchActions = shallowRef<BookDetailSearchResultActions | null>(null);
  const searchResultBatchBusy = computed(() => activeSearchActions.value?.searchResultBatchBusy.value ?? false);

  function searchResultIsBusy(result: SearchBookResult): boolean {
    return activeSearchActions.value?.searchResultIsBusy(result) ?? false;
  }

  const searchResultDetailsShelfBook = computed(() => {
    const bookId = dependencies.openedBook.value?.id;
    return bookId ? dependencies.shelf.value.books.find((book) => book.id === bookId) ?? null : null;
  });
  watch(searchResultDetailsShelfBook, (book) => {
    if (book && searchResultDetails.value) searchResultDetails.value.inShelf = true;
  });

  function isCurrentRequest(requestGeneration: number, restoreRevision: number, details: ActiveBookDetailSession): boolean {
    return requestGeneration === generation
      && restoreRevision === dependencies.getRestoreRevision()
      && searchResultDetails.value === details;
  }

  function beginSession(state: BookDetailSessionState, searchActions?: BookDetailSearchResultActions): {
    generation: number;
    restoreRevision: number;
    details: ActiveBookDetailSession;
  } {
    generation += 1;
    const currentGeneration = generation;
    const restoreRevision = dependencies.getRestoreRevision();
    activeSearchActions.value = searchActions ?? null;
    dependencies.closeDisplayEditor();
    searchResultDetailsReadingBusy.value = false;
    bookPanelBusy.value = true;
    dependencies.openedBook.value = null;
    const details = reactive(state);
    searchResultDetails.value = details;
    return { generation: currentGeneration, restoreRevision, details };
  }

  async function openSearchResultDetails(
    result: SearchBookResult,
    searchActions: BookDetailSearchResultActions,
  ): Promise<void> {
    const request = beginSession({
      result,
      bookId: null,
      inShelf: false,
      initialTab: "info",
      error: "",
    }, searchActions);
    try {
      const response = await prepareSearchResultBook(result.resultId);
      const book = await readResource<BookResource>(response.book);
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      request.details.bookId = book.id;
      dependencies.openedBook.value = book;
    } catch (error) {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) {
        request.details.error = dependencies.errorText(error);
      }
    } finally {
      if (isCurrentRequest(request.generation, request.restoreRevision, request.details)) bookPanelBusy.value = false;
    }
  }

  async function openShelfBook(bookId: string, initialTab: BookDetailTab = "catalog"): Promise<void> {
    const request = beginSession({
      result: null,
      bookId,
      inShelf: true,
      initialTab,
      error: "",
    });
    try {
      if (!dependencies.shelf.value.books.some((entry) => entry.id === bookId)) {
        throw new Error("\u4e66\u7c4d\u6587\u4ef6\u6682\u65f6\u4e0d\u53ef\u7528\u3002");
      }
      const book = await readResource<BookResource>(await getBook(bookId));
      if (!isCurrentRequest(request.generation, request.restoreRevision, request.details)) return;
      dependencies.openedBook.value = book;
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
      await openSearchResultDetails(request.result, request.actions);
      return;
    }
    await openShelfBook(request.bookId, request.initialTab);
  }

  async function retry(): Promise<void> {
    const details = searchResultDetails.value;
    if (!details) return;
    if (details.result && activeSearchActions.value) {
      await openSearchResultDetails(details.result, activeSearchActions.value);
      return;
    }
    if (details.bookId) await openShelfBook(details.bookId, details.initialTab);
  }

  async function startReading(chapterIndex?: number): Promise<void> {
    const details = searchResultDetails.value;
    const book = dependencies.openedBook.value;
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
    const details = searchResultDetails.value;
    const result = details?.result;
    const addSearchResult = activeSearchActions.value?.addSearchResult;
    if (!details || !result || !dependencies.openedBook.value || searchResultDetailsShelfBook.value || !addSearchResult) return;
    const requestGeneration = generation;
    const restoreRevision = dependencies.getRestoreRevision();
    await addSearchResult(result);
    if (!isCurrentRequest(requestGeneration, restoreRevision, details)) return;
    await dependencies.refreshShelf();
    if (!isCurrentRequest(requestGeneration, restoreRevision, details) || !details.bookId
      || !searchResultDetailsShelfBook.value) return;
    details.inShelf = true;
    bookPanelBusy.value = true;
    try {
      if (!dependencies.shelf.value.books.some((entry) => entry.id === details.bookId)) {
        throw new Error("\u4e66\u7c4d\u6587\u4ef6\u6682\u65f6\u4e0d\u53ef\u7528\u3002");
      }
      const book = await readResource<BookResource>(await getBook(details.bookId));
      if (!isCurrentRequest(requestGeneration, restoreRevision, details)) return;
      dependencies.openedBook.value = book;
    } catch (error) {
      if (isCurrentRequest(requestGeneration, restoreRevision, details)) {
        dependencies.notify(`\u4e66\u7c4d\u5df2\u52a0\u5165\u4e66\u67b6\uff0c\u8be6\u60c5\u5237\u65b0\u5931\u8d25\uff1a${dependencies.errorText(error)}`, "error");
      }
    } finally {
      if (isCurrentRequest(requestGeneration, restoreRevision, details)) bookPanelBusy.value = false;
    }
  }

  function clear(options: { preserveOpenedBook?: boolean } = {}): void {
    generation += 1;
    activeSearchActions.value = null;
    bookPanelBusy.value = false;
    searchResultDetailsReadingBusy.value = false;
    searchResultDetails.value = null;
    if (!options.preserveOpenedBook) dependencies.openedBook.value = null;
    dependencies.closeDisplayEditor();
  }

  return {
    searchResultDetails,
    searchResultDetailsShelfBook,
    searchResultDetailsReadingBusy,
    searchResultBatchBusy,
    searchResultIsBusy,
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
