import { computed, ref, type Ref } from "vue";
import { type AppTask, type BookResource, type ResourceDescriptor, type SearchBookResult, type SearchResource, type SourceMetadata, type TaskResponse } from "../../api/types";
import { cancelTask, checkNewChapters, refreshChapters, startBookDownload } from "../../api/tasks";
import { readResource } from "../../api/resources";
import { startSearch } from "../../api/search";

interface SearchWorkflowDependencies {
  bookSourceCandidates: Readonly<Ref<SourceMetadata[]>>;
  selectedSourceIds: Ref<string[]>;
  searchKeyword: Ref<string>;
  searchPage: Ref<number>;
  searchBusy: Ref<boolean>;
  searchProgress: Ref<string>;
  searchResponseErrors: Ref<Array<{ sourceId?: string; message: string }>>;
  searchResults: Ref<SearchBookResult[]>;
  searchResourceErrors: Ref<Array<{ sourceId?: string; message: string }>>;
  searchWasRun: Ref<boolean>;
  searchResultBatchBusy: Ref<boolean>;
  shelfBatchRecoveryRequired: Readonly<Ref<boolean>>;
  openedBook?: Ref<BookResource | null>;
  resetSearchResultState: () => void;
  pruneSelectionToResults: (results: SearchBookResult[]) => void;
  refreshTasks?: (descriptor?: ResourceDescriptor) => Promise<void>;
  reusedCatalogTaskNotice?: (task: AppTask) => string;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  getRestoreRevision: () => number;
  startSearch: typeof startSearch;
  cancelTask: typeof cancelTask;
  startBookDownload?: typeof startBookDownload;
  refreshChapters?: typeof refreshChapters;
  checkNewChapters?: typeof checkNewChapters;
  readResource: typeof readResource;
}

/**
 * 搜索业务状态只来自 Rust 发布的完整、不可变快照。
 * 新 descriptor 立即替换展示资源的读取，Rust 负责业务结果与发布权。
 */
export function useSearchWorkflow({
  bookSourceCandidates,
  selectedSourceIds,
  searchKeyword,
  searchPage,
  searchBusy,
  searchProgress,
  searchResponseErrors,
  searchResults,
  searchResourceErrors,
  searchWasRun,
  searchResultBatchBusy,
  shelfBatchRecoveryRequired,
  openedBook,
  resetSearchResultState,
  pruneSelectionToResults,
  refreshTasks,
  reusedCatalogTaskNotice,
  notify,
  errorText,
  getRestoreRevision,
  startSearch,
  cancelTask,
  startBookDownload,
  refreshChapters,
  checkNewChapters,
  readResource,
}: SearchWorkflowDependencies) {
  const activeSearchTaskId = ref<string | null>(null);
  const searchCancelRequestTaskId = ref<string | null>(null);
  const activeSearchCancellationPending = computed(
    () => activeSearchTaskId.value !== null
      && searchCancelRequestTaskId.value === activeSearchTaskId.value,
  );

  // 仅取消旧展示资源的 HTTP 读取；不取消或仲裁 Rust 的搜索生产者。
  let snapshotRead: AbortController | null = null;
  let searchStartPending = false;
  let displayedQuery: { keyword: string; sourceIds: string[] } | null = null;

  async function applySearchSnapshot(descriptor: ResourceDescriptor, restoreRevision: number, signal: AbortSignal): Promise<void> {
    if (signal.aborted || restoreRevision !== getRestoreRevision()) return;
    const result = await readResource<SearchResource>(descriptor, signal);
    if (signal.aborted || restoreRevision !== getRestoreRevision()) return;
    const nextResults = Array.isArray(result.results) ? result.results : [];
    const status = result.status ?? (result.complete === true ? "completed" : "running");
    displayedQuery = { keyword: result.keyword, sourceIds: result.sourceIds ?? [] };

    searchKeyword.value = result.keyword;
    searchWasRun.value = true;
    searchPage.value = Math.max(1, result.page || 1);
    searchResults.value = nextResults;
    searchResourceErrors.value = Array.isArray(result.errors) ? result.errors : [];
    searchResponseErrors.value = status === "failed" && result.error ? [{ message: result.error }] : [];

    if (result.complete === true) pruneSelectionToResults(nextResults);

    if (status === "queued" || status === "running") {
      if (result.taskId) activeSearchTaskId.value = result.taskId;
      searchBusy.value = true;
      const completed = result.completedSources ?? 0;
      const total = result.totalSources ?? 0;
      searchProgress.value = total > 0
        ? `正在搜索「${result.keyword}」（${completed}/${total}）…`
        : `正在搜索「${result.keyword}」…`;
      return;
    }

    if (status === "paused") {
      if (result.taskId) activeSearchTaskId.value = result.taskId;
      searchBusy.value = false;
      searchProgress.value = "搜索已暂停。";
      return;
    }

    searchBusy.value = false;
    activeSearchTaskId.value = null;
    searchCancelRequestTaskId.value = null;
    if (status === "completed") {
      searchProgress.value = `「${result.keyword}」搜索完成 · ${nextResults.length} 本书`;
    } else if (status === "cancelled") {
      searchProgress.value = "搜索已取消。";
    } else if (status === "failed") {
      searchProgress.value = "搜索失败";
      if (result.error) notify(`搜索失败：${result.error}`, "error");
    }
  }

  async function consumeSearchSnapshot(descriptor: ResourceDescriptor): Promise<void> {
    snapshotRead?.abort();
    const controller = new AbortController();
    snapshotRead = controller;
    const revision = getRestoreRevision();
    try {
      await applySearchSnapshot(descriptor, revision, controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) throw error;
    }
  }

  async function runSearch(page?: number): Promise<void> {
    if (shelfBatchRecoveryRequired.value || searchResultBatchBusy.value || searchStartPending) return;
    const restoreRevision = getRestoreRevision();
    const keyword = (page === undefined ? searchKeyword.value : displayedQuery?.keyword ?? searchKeyword.value).trim();
    const nextPage = Math.max(1, page ?? 1);
    if (!keyword) {
      notify("先输入书名、作者或关键词。", "error");
      return;
    }
    const eligibleSourceIds = new Set(bookSourceCandidates.value.map((source) => source.id));
    const querySourceIds = page === undefined ? selectedSourceIds.value : displayedQuery?.sourceIds ?? selectedSourceIds.value;
    const sourceIds = querySourceIds.filter((id) => eligibleSourceIds.has(id));
    if (page === undefined && sourceIds.length !== selectedSourceIds.value.length) selectedSourceIds.value = sourceIds;
    if (!sourceIds.length) {
      notify("请至少选择一个已启用书源。", "error");
      return;
    }

    // 新的一次交互立即离开上一轮展示；旧结果在 Rust 接受新搜索后即失效。
    snapshotRead?.abort();
    snapshotRead = null;
    resetSearchResultState();
    searchResults.value = [];
    searchResourceErrors.value = [];
    searchResponseErrors.value = [];
    searchWasRun.value = true;
    searchBusy.value = true;
    searchPage.value = nextPage;
    searchProgress.value = "正在启动搜索…";
    activeSearchTaskId.value = null;
    searchCancelRequestTaskId.value = null;
    searchStartPending = true;

    try {
      const response = await startSearch(sourceIds, keyword, nextPage);
      if (restoreRevision !== getRestoreRevision()) return;
      if (!response.taskId) throw new Error("搜索任务没有返回有效编号。");
      // 首份资源尚未显示时，让启动中的交互可以取消；结果仍只来自资源。
      if (activeSearchTaskId.value === null && searchBusy.value) activeSearchTaskId.value = response.taskId;
    } catch (error) {
      if (restoreRevision !== getRestoreRevision()) return;
      if (snapshotRead === null) {
        searchBusy.value = false;
        searchProgress.value = "搜索启动失败。";
      }
      notify(`搜索失败：${errorText(error)}`, "error");
    } finally {
      searchStartPending = false;
    }
  }

  async function cancelActiveSearch(): Promise<void> {
    const taskId = activeSearchTaskId.value;
    const restoreRevision = getRestoreRevision();
    if (!taskId || !searchBusy.value || searchCancelRequestTaskId.value === taskId) return;
    searchCancelRequestTaskId.value = taskId;
    searchProgress.value = "正在取消搜索…";
    try {
      await cancelTask(taskId);
      // 最终 cancelled 状态仍由 Rust 的搜索快照发布，command response 不参与结果仲裁。
    } catch (error) {
      if (restoreRevision === getRestoreRevision() && activeSearchTaskId.value === taskId) {
        searchCancelRequestTaskId.value = null;
        searchProgress.value = "取消搜索失败，搜索仍在进行。";
        notify(`取消搜索失败：${errorText(error)}`, "error");
      }
    }
  }

  async function enqueueBookTask(action: "download" | "refresh" | "check"): Promise<void> {
    if (shelfBatchRecoveryRequired.value || !openedBook || !refreshTasks || !reusedCatalogTaskNotice
      || !startBookDownload || !refreshChapters || !checkNewChapters) return;
    const book = openedBook.value;
    const restoreRevision = getRestoreRevision();
    if (!book) return;
    try {
      let response: TaskResponse;
      if (action === "download") {
        response = await startBookDownload(book.id);
      } else if (action === "refresh") {
        response = await refreshChapters(book.id);
      } else {
        response = await checkNewChapters(book.id);
      }
      if (restoreRevision !== getRestoreRevision()) return;
      await refreshTasks(response.resource);
      if (restoreRevision !== getRestoreRevision()) return;
      if (response.reused) {
        notify(reusedCatalogTaskNotice(response.task));
        return;
      }
      notify(action === "download" ? "整本缓存任务已加入。" : action === "refresh" ? "目录更新任务已加入。" : "新章节检查已加入。 ");
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) {
        notify(`无法创建任务：${errorText(error)}`, "error");
      }
    }
  }

  function resetForRestore(): void {
    snapshotRead?.abort();
    snapshotRead = null;
    displayedQuery = null;
    activeSearchTaskId.value = null;
    searchCancelRequestTaskId.value = null;
    searchBusy.value = false;
    searchWasRun.value = false;
    searchProgress.value = "";
    searchResponseErrors.value = [];
    searchResourceErrors.value = [];
    searchResults.value = [];
    resetSearchResultState();
  }

  return {
    resetForRestore,
    activeSearchTaskId,
    activeSearchCancellationPending,
    consumeSearchSnapshot,
    runSearch,
    cancelActiveSearch,
    enqueueBookTask,
  };
}
