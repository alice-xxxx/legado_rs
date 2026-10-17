import { computed, ref, shallowRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { startSearch, stopSearch, releaseSearchResults } from "../../api/search";
import router, { routeNames } from "../../router";
import type { AppBootstrap, SearchBookResult, SearchBatch, SearchType } from "../../api/types";
import { bookSourceCandidates } from "../sources/sourcesStore";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";

export const searchType = ref<SearchType>("text");
export const selectedSourceIds = ref<string[]>([]);
export const searchKeyword = ref("");
export const precisionSearch = ref(true);
export const searchPage = ref(1);
export const searchBusy = ref(false);
export const searchHasMore = ref(false);
export const searchComplete = ref(false);
export const searchProgress = ref("");
export const searchResults = shallowRef<SearchBookResult[]>([]);
export const searchResponseErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
export const searchResourceErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
export const searchWasRun = ref(false);
export const activeSearchTaskId = ref<string | null>(null);
export const activeSearchCancellationPending = ref(false);
export const searchSources = computed(() => bookSourceCandidates.value.filter(source =>
  source.searchType === searchType.value));

let retainedTaskId: string | null = null;
let generation = 0;
interface SearchRun {
  generation: number;
  taskId: string | null;
  stopRequested: boolean;
  finished: boolean;
  stopPromise?: Promise<void>;
}
let currentRun: SearchRun | null = null;
// Serialize command admission so a late start cannot replace a newer search in Rust.
let startQueue: Promise<void> = Promise.resolve();
let initialization: Promise<void> | null = null;
let unlisten: UnlistenFn | null = null;
let removeRouteHook: (() => void) | null = null;
let displayedQuery: { type: SearchType; keyword: string; sourceIds: string[]; precision: boolean } | null = null;
let previousType: SearchType | null = null;
let previousIds: string[] = [];

watch([searchType, () => searchSources.value.map(source => source.id)], ([type, ids]) => {
  if (previousType !== type) {
    resetSearch();
    selectedSourceIds.value = [...ids];
  } else {
    const eligible = new Set(ids);
    selectedSourceIds.value = [...selectedSourceIds.value.filter(id => eligible.has(id)),
      ...ids.filter(id => !previousIds.includes(id))];
  }
  previousType = type;
  previousIds = [...ids];
}, { immediate: true });

watch(searchKeyword, keyword => {
  if (!keyword.trim()) { resetSearch(); return; }
  if (displayedQuery && keyword.trim() !== displayedQuery.keyword) void cancelActiveSearch();
}, { flush: "sync" });

export function initializeSearchState(): Promise<void> {
  if (initialization) return initialization;
  removeRouteHook = router.afterEach((to, from, failure) => {
    if (failure || to.name === routeNames.search || to.name === routeNames.bookDetail) return;
    // A search-result detail page is part of the same search flow. Leaving it
    // for any other destination also stops the discovery worker.
    if (from.name === routeNames.search || from.name === routeNames.bookDetail) {
      void cancelActiveSearch();
    }
  });
  const pending = listen<AppBootstrap>("app-state-updated", resetSearch).then(dispose => {
    if (initialization === pending) unlisten = dispose;
    else dispose();
  }).catch(error => {
    if (initialization === pending) {
      removeRouteHook?.();
      removeRouteHook = null;
      initialization = null;
    }
    throw error;
  });
  initialization = pending;
  return pending;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function applyBatch(batch: SearchBatch, run: SearchRun): void {
  if (currentRun !== run || generation !== run.generation || run.finished) return;
  if (run.taskId && batch.taskId !== run.taskId) return;
  if (batch.taskId) {
    run.taskId = batch.taskId;
    activeSearchTaskId.value = batch.taskId;
    retainedTaskId = batch.taskId;
  }
  const status = batch.status ?? (batch.complete ? "completed" : "running");
  const terminal = Boolean(batch.complete || ["completed", "failed", "cancelled"].includes(status));
  if (run.stopRequested && !terminal) {
    void sendStopRequest(run);
    return;
  }
  const results = Array.isArray(batch.results) ? batch.results : [];
  if (batch.incremental && results.length) {
    const merged = new Map(searchResults.value.map(result => [result.resultId, result]));
    for (const result of results) merged.set(result.resultId, result);
    searchResults.value = batch.resultOrder.map(id => merged.get(id))
      .filter((result): result is SearchBookResult => result !== undefined);
  } else if (!batch.incremental) searchResults.value = results;
  if (!batch.incrementalErrors) searchResourceErrors.value = batch.errors ?? [];
  else if (batch.errors?.length) searchResourceErrors.value = [...searchResourceErrors.value, ...batch.errors];
  searchHasMore.value = status === "completed" && batch.hasMore === true && !run.stopRequested;
  searchComplete.value = status === "completed" && !run.stopRequested;
  searchBusy.value = status === "queued" || status === "running";
  searchProgress.value = searchBusy.value
    ? `正在搜索（${batch.completedSources ?? 0}/${batch.totalSources ?? 0}）…`
    : status === "completed" ? "搜索完成"
    : status === "cancelled" ? "搜索已取消。"
    : status === "paused" ? "搜索已暂停。" : "搜索失败。";
  if (batch.error) searchResponseErrors.value = [{ message: batch.error }];
  if (terminal) {
    run.finished = true;
    activeSearchTaskId.value = null;
    activeSearchCancellationPending.value = false;
  }
}

export async function runSearch(page?: number): Promise<void> {
  if (shelfBatchRecoveryRequired.value) return;
  const type = page === undefined ? searchType.value : displayedQuery?.type ?? searchType.value;
  const keyword = (page === undefined ? searchKeyword.value : displayedQuery?.keyword ?? searchKeyword.value).trim();
  const precision = page === undefined ? precisionSearch.value : displayedQuery?.precision ?? precisionSearch.value;
  const eligible = new Set(searchSources.value.map(source => source.id));
  const sourceIds = (page === undefined ? selectedSourceIds.value : displayedQuery?.sourceIds ?? selectedSourceIds.value)
    .filter(id => eligible.has(id));
  if (!keyword || !sourceIds.length) {
    notify(!keyword ? "先输入书名、作者或关键词。" : "请至少选择一个当前类型的已启用源。", "error");
    return;
  }
  const requestGeneration = ++generation;
  const run: SearchRun = { generation: requestGeneration, taskId: null, stopRequested: false, finished: false };
  currentRun = run;
  const nextPage = Math.max(1, page ?? 1);
  displayedQuery = { type, keyword, sourceIds, precision };
  if (nextPage === 1) searchResults.value = [];
  searchResponseErrors.value = [];
  searchResourceErrors.value = [];
  activeSearchTaskId.value = null;
  activeSearchCancellationPending.value = false;
  searchWasRun.value = true;
  searchBusy.value = true;
  searchHasMore.value = false;
  searchComplete.value = false;
  searchPage.value = nextPage;
  searchProgress.value = "正在启动搜索…";
  const admission = startQueue.then(async () => {
    await initializeSearchState();
    if (currentRun !== run || run.stopRequested) {
      if (currentRun === run) {
        run.finished = true;
        searchBusy.value = false;
        activeSearchCancellationPending.value = false;
        searchProgress.value = "搜索已停止。";
      }
      return;
    }
    const response = await startSearch(type, sourceIds, keyword, nextPage, precision,
      batch => applyBatch(batch, run));
    run.taskId = response.taskId;
    if (currentRun !== run) { await releaseSearchResults(response.taskId); return; }
    retainedTaskId = response.taskId;
    if (run.stopRequested) await sendStopRequest(run);
    else if (!run.finished) activeSearchTaskId.value = response.taskId;
  });
  startQueue = admission.catch(() => {});
  try {
    await admission;
  } catch (error) {
    if (currentRun !== run) return;
    run.finished = true;
    searchBusy.value = false;
    activeSearchTaskId.value = null;
    activeSearchCancellationPending.value = false;
    searchProgress.value = "搜索启动失败。";
    notify(errorText(error), "error");
  }
}

function sendStopRequest(run: SearchRun): Promise<void> {
  if (!run.taskId || run.finished) return Promise.resolve();
  if (run.stopPromise) return run.stopPromise;
  run.stopPromise = stopSearch(run.taskId).then(() => {
    if (currentRun !== run || run.finished) return;
    run.finished = true;
    searchBusy.value = false;
    activeSearchCancellationPending.value = false;
    activeSearchTaskId.value = null;
    searchProgress.value = "搜索已停止。";
  }).catch(error => {
    if (currentRun !== run || run.finished) return;
    run.stopRequested = false;
    run.stopPromise = undefined;
    activeSearchCancellationPending.value = false;
    searchBusy.value = true;
    notify(`停止搜索失败：${errorText(error)}`, "error");
  });
  return run.stopPromise;
}

export function cancelActiveSearch(): Promise<void> {
  const run = currentRun;
  if (!run || run.finished) return Promise.resolve();
  run.stopRequested = true;
  searchComplete.value = false;
  searchHasMore.value = false;
  activeSearchCancellationPending.value = true;
  searchProgress.value = "正在停止搜索…";
  // The first IPC batch or start response will stop a task whose ID is not yet known.
  return sendStopRequest(run);
}

export function resetSearch(): void {
  const taskId = activeSearchTaskId.value ?? retainedTaskId;
  retainedTaskId = null;
  currentRun = null;
  searchComplete.value = false;
  if (taskId) void releaseSearchResults(taskId).catch(console.error);
  generation++;
  displayedQuery = null;
  activeSearchTaskId.value = null;
  activeSearchCancellationPending.value = false;
  searchBusy.value = false;
  searchHasMore.value = false;
  searchWasRun.value = false;
  searchProgress.value = "";
  searchResults.value = [];
  searchResponseErrors.value = [];
  searchResourceErrors.value = [];
  searchPage.value = 1;
}

export function disposeSearchState(): void {
  resetSearch();
  unlisten?.();
  unlisten = null;
  removeRouteHook?.();
  removeRouteHook = null;
  initialization = null;
}

export function loadMoreSearchResults(): Promise<void> {
  if (searchBusy.value || activeSearchCancellationPending.value || !searchHasMore.value) return Promise.resolve();
  return runSearch(searchPage.value + 1);
}
