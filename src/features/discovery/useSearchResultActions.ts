import { computed, ref, type Ref } from "vue";
import type { BookMutationResponse, BookResource, ResourceDescriptor, SearchBookResult, ShelfResource } from "../../api/types";

type NoticeKind = "success" | "error";
type AddOutcome = "added" | "already" | "failed" | "stale";

interface SearchResultActionsOptions {
  /** 搜索任务与结果仍由发现功能的宿主管理。 */
  searchResults: Ref<SearchBookResult[]>;
  /** 书架、恢复状态及详情弹层属于应用级状态，由宿主显式注入。 */
  shelf: Ref<ShelfResource>;
  shelfBatchRecoveryRequired: Readonly<Ref<boolean>>;
  shelfBatchRecoveryMessage: Readonly<Ref<string>>;
  getRestoreRevision: () => number;
  addBook: (resultId: string) => Promise<BookMutationResponse>;
  readBookResource: (descriptor: ResourceDescriptor) => Promise<BookResource>;
  readShelfResource: (descriptor: ResourceDescriptor) => Promise<ShelfResource>;
  errorText: (error: unknown) => string;
  notify: (message: string, kind?: NoticeKind) => void;
}

/** 管理书籍搜索结果的选择与加入书架；搜索结果、书架和恢复锁由宿主注入。 */
export function useSearchResultActions(options: SearchResultActionsOptions) {
  const selectedSearchResultIds = ref<string[]>([]);
  const searchResultBatchBusy = ref(false);
  const searchResultBatchSummary = ref("");
  const searchResultAddErrors = ref<Record<string, string>>({});
  const busySearchResultKeys = ref<string[]>([]);
  const addedSearchResultBookIds = ref<Record<string, string>>({});

  const selectedSearchResults = computed(() => {
    const selectedIds = new Set(selectedSearchResultIds.value);
    return options.searchResults.value.filter((result) => selectedIds.has(searchResultSelectionKey(result)));
  });

  function searchResultIdentityKey(result: SearchBookResult): string {
    return `${result.sourceId}\u0000${result.bookUrl || result.resultId}`;
  }

  function searchResultSelectionKey(result: SearchBookResult): string {
    return searchResultIdentityKey(result);
  }

  function searchResultIsBusy(result: SearchBookResult): boolean {
    return busySearchResultKeys.value.includes(searchResultIdentityKey(result));
  }

  function searchResultIsAdded(result: SearchBookResult): boolean {
    const bookId = addedSearchResultBookIds.value[searchResultIdentityKey(result)];
    return Boolean(bookId && options.shelf.value.books.some((book) => book.id === bookId));
  }

  function resetSearchResultState(): void {
    selectedSearchResultIds.value = [];
    searchResultAddErrors.value = {};
    searchResultBatchSummary.value = "";
  }

  function pruneSelectionToResults(results: SearchBookResult[]): void {
    const availableKeys = new Set(results.map(searchResultSelectionKey));
    selectedSearchResultIds.value = selectedSearchResultIds.value.filter((key) => availableKeys.has(key));
  }

  function toggleSearchResultSelection(result: SearchBookResult): void {
    if (searchResultBatchBusy.value || options.shelfBatchRecoveryRequired.value
      || searchResultIsAdded(result)) return;
    const selectionKey = searchResultSelectionKey(result);
    selectedSearchResultIds.value = selectedSearchResultIds.value.includes(selectionKey)
      ? selectedSearchResultIds.value.filter((id) => id !== selectionKey)
      : [...selectedSearchResultIds.value, selectionKey];
  }

  function toggleVisibleSearchResultSelection(visibleResults: SearchBookResult[]): void {
    if (searchResultBatchBusy.value || options.shelfBatchRecoveryRequired.value) return;
    const visibleIds = visibleResults.map(searchResultSelectionKey);
    if (!visibleIds.length) return;
    const visibleIdSet = new Set(visibleIds);
    const allVisibleSelected = visibleIds.every((id) => selectedSearchResultIds.value.includes(id));
    selectedSearchResultIds.value = allVisibleSelected
      ? selectedSearchResultIds.value.filter((id) => !visibleIdSet.has(id))
      : [...new Set([...selectedSearchResultIds.value, ...visibleIds])];
  }

  function clearSearchResultSelection(): void {
    if (searchResultBatchBusy.value) return;
    selectedSearchResultIds.value = [];
  }

  async function addSearchResultToShelf(result: SearchBookResult): Promise<AddOutcome> {
    const identityKey = searchResultIdentityKey(result);
    const selectionKey = searchResultSelectionKey(result);
    if (options.shelfBatchRecoveryRequired.value) {
      searchResultAddErrors.value = {
        ...searchResultAddErrors.value,
        [selectionKey]: options.shelfBatchRecoveryMessage.value || "应用正在恢复，尚未发起加入操作。",
      };
      return "failed";
    }
    if (searchResultIsBusy(result)) {
      searchResultAddErrors.value = {
        ...searchResultAddErrors.value,
        [selectionKey]: "这本书已有加入操作正在进行。",
      };
      return "failed";
    }
    if (searchResultIsAdded(result)) {
      const nextErrors = { ...searchResultAddErrors.value };
      delete nextErrors[selectionKey];
      searchResultAddErrors.value = nextErrors;
      return "already";
    }

    const restoreRevision = options.getRestoreRevision();
    busySearchResultKeys.value = [...busySearchResultKeys.value, identityKey];
    const existingBookIds = new Set(options.shelf.value.books.map((book) => book.id));
    try {
      const response = await options.addBook(result.resultId);
      if (restoreRevision !== options.getRestoreRevision()) return "stale";
      const [book, nextShelf] = await Promise.all([
        options.readBookResource(response.book),
        options.readShelfResource(response.shelf),
      ]);
      if (restoreRevision !== options.getRestoreRevision()) return "stale";
      options.shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
      addedSearchResultBookIds.value = { ...addedSearchResultBookIds.value, [identityKey]: book.id };
      const nextErrors = { ...searchResultAddErrors.value };
      delete nextErrors[selectionKey];
      searchResultAddErrors.value = nextErrors;
      return existingBookIds.has(book.id) ? "already" : "added";
    } catch (error) {
      if (restoreRevision !== options.getRestoreRevision()) return "stale";
      searchResultAddErrors.value = { ...searchResultAddErrors.value, [selectionKey]: options.errorText(error) };
      return "failed";
    } finally {
      busySearchResultKeys.value = busySearchResultKeys.value.filter((key) => key !== identityKey);
    }
  }

  async function addSearchResult(result: SearchBookResult): Promise<void> {
    if (searchResultBatchBusy.value || options.shelfBatchRecoveryRequired.value
      || searchResultIsBusy(result) || searchResultIsAdded(result)) return;
    const outcome = await addSearchResultToShelf(result);
    if (outcome === "stale") return;
    if (outcome === "failed") {
      options.notify(
        `加入《${result.title}》失败（${result.sourceName}）：${searchResultAddErrors.value[searchResultSelectionKey(result)] || "请重试。"}`,
        "error",
      );
      return;
    }
    const selectionKey = searchResultSelectionKey(result);
    selectedSearchResultIds.value = selectedSearchResultIds.value.filter((id) => id !== selectionKey);
    options.notify(outcome === "already"
      ? `《${result.title}》已在书架，现有阅读数据已保留。`
      : `《${result.title}》已加入书架。`);
  }

  async function addSelectedSearchResults(): Promise<void> {
    if (searchResultBatchBusy.value || options.shelfBatchRecoveryRequired.value) return;
    const selectedIds = new Set(selectedSearchResultIds.value);
    const targets = options.searchResults.value.filter((result) => selectedIds.has(searchResultSelectionKey(result)));
    if (!targets.length) return;
    if (busySearchResultKeys.value.length) {
      options.notify("请等正在进行的单本加入操作完成后，再批量加入。", "error");
      return;
    }

    const restoreRevision = options.getRestoreRevision();
    searchResultBatchBusy.value = true;
    searchResultBatchSummary.value = "正在批量加入所选书籍…";
    let added = 0;
    let already = 0;
    let failed = 0;
    let notAttempted = 0;
    const completedResultIds = new Set<string>();
    try {
      for (let index = 0; index < targets.length; index += 1) {
        if (restoreRevision !== options.getRestoreRevision()) return;
        if (options.shelfBatchRecoveryRequired.value) {
          notAttempted = targets.length - index;
          break;
        }
        const result = targets[index];
        const selectionKey = searchResultSelectionKey(result);
        const outcome = await addSearchResultToShelf(result);
        if (outcome === "stale" || restoreRevision !== options.getRestoreRevision()) return;
        if (outcome === "added") {
          added += 1;
          completedResultIds.add(selectionKey);
        } else if (outcome === "already") {
          already += 1;
          completedResultIds.add(selectionKey);
        } else {
          failed += 1;
        }
        searchResultBatchSummary.value = `正在加入：新加入 ${added} 本，已在书架 ${already} 本，失败 ${failed} 本。`;
      }
    } finally {
      searchResultBatchBusy.value = false;
    }

    if (restoreRevision !== options.getRestoreRevision()) return;
    selectedSearchResultIds.value = selectedSearchResultIds.value.filter((id) => !completedResultIds.has(id));
    const recoveryNote = notAttempted
      ? `；恢复处理中，${notAttempted} 本未尝试并保留选择`
      : "";
    searchResultBatchSummary.value = `批量加入结果：新加入 ${added} 本，已在书架 ${already} 本，失败 ${failed} 本${recoveryNote}。`;
    options.notify(searchResultBatchSummary.value, failed || notAttempted ? "error" : "success");
  }

  return {
    selectedSearchResultIds,
    searchResultBatchBusy,
    searchResultBatchSummary,
    searchResultAddErrors,
    busySearchResultKeys,
    selectedSearchResults,
    searchResultSelectionKey,
    searchResultIsBusy,
    searchResultIsAdded,
    resetSearchResultState,
    pruneSelectionToResults,
    toggleSearchResultSelection,
    toggleVisibleSearchResultSelection,
    clearSearchResultSelection,
    addSearchResult,
    addSelectedSearchResults,
  };
}
