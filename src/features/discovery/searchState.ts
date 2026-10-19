import { computed, ref, shallowRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { search, stopSearch, releaseSearchResults } from "../../api/search";
import router, { routeNames } from "../../router";
import type { AppBootstrap, SearchBookResult, SearchResultSource, SearchUpdate, SearchPrecision, SearchType } from "../../api/types";
import { bookSourceCandidates } from "../sources/sourcesStore";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { presentSearchResults } from "./searchPresentation";

export const searchType = ref<SearchType>("text");
export const selectedSourceIds = ref<string[]>([]);
export const searchKeyword = ref("");
export const precisionSearch = ref<SearchPrecision>("precise");
export const searchPage = ref(1);
export const searchBusy = ref(false);
export const searchHasMore = ref(false);
export const searchComplete = ref(false);
export const searchProgress = ref("");
export const searchResults = shallowRef<SearchBookResult[]>([]);
export const searchErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
export const searchWasRun = ref(false);
export const activeSearchCancellationPending = ref(false);
export const searchSources = computed(() => bookSourceCandidates.value.filter(source =>
  source.searchType === searchType.value));

interface SearchRun {
  searchId: string | null;
  stopRequested: boolean;
  finished: boolean;
  results: SearchResultSource[];
  completedSources: number;
  stopPromise?: Promise<void>;
}
let currentRun: SearchRun | null = null;
// 翻页沿用已收到的结果，关闭或重新搜索时统一释放原始数据。
const retainedSearchIds = new Set<string>();
let initialization: Promise<void> | null = null;
let unlisten: UnlistenFn | null = null;
let removeRouteHook: (() => void) | null = null;
let displayedQuery: { keyword: string; sourceIds: string[]; precision: SearchPrecision; nextSourceIds: string[] } | null = null;
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

function applyBatch(batch: SearchUpdate, run: SearchRun): void {
  if (currentRun !== run) {
    void releaseSearchResults(batch.searchId).catch(console.error);
    return;
  }
  if (run.finished) return;
  if (run.searchId && batch.searchId !== run.searchId) return;
  run.searchId = batch.searchId;
  retainedSearchIds.add(batch.searchId);
  if (run.stopRequested) { void cancelActiveSearch(); return; }
  run.results.push(...batch.results);
  if (batch.sourceId) {
    run.completedSources++;
    if (batch.hasMore) displayedQuery?.nextSourceIds.push(batch.sourceId);
  }
  searchResults.value = presentSearchResults(run.results, displayedQuery!.keyword);
  if (batch.error) searchErrors.value.push({ sourceId: batch.sourceId, message: batch.error });
  searchProgress.value = `已找到 ${searchResults.value.length} 本，搜索中（${run.completedSources}/${displayedQuery!.sourceIds.length}）…`;
}

export async function runSearch(page?: number): Promise<void> {
  if (shelfBatchRecoveryRequired.value) return;
  const keyword = (page === undefined ? searchKeyword.value : displayedQuery?.keyword ?? searchKeyword.value).trim();
  const precision = page === undefined ? precisionSearch.value : displayedQuery?.precision ?? precisionSearch.value;
  const eligible = new Set(searchSources.value.map(source => source.id));
  const sourceIds = (page === undefined ? selectedSourceIds.value : displayedQuery?.nextSourceIds ?? selectedSourceIds.value)
    .filter(id => eligible.has(id));
  if (!keyword || !sourceIds.length) {
    notify(!keyword ? "先输入书名、作者或关键词。" : "请至少选择一个当前类型的已启用源。", "error");
    return;
  }
  const previousRun = currentRun;
  if (previousRun && !previousRun.finished) await cancelActiveSearch();
  if (currentRun !== previousRun) return;
  if (page === undefined) {
    for (const id of retainedSearchIds) void releaseSearchResults(id).catch(console.error);
    retainedSearchIds.clear();
  }
  const run: SearchRun = { searchId: null, stopRequested: false, finished: false,
    results: page === undefined ? [] : currentRun?.results.slice() ?? [], completedSources: 0 };
  currentRun = run;
  const nextPage = Math.max(1, page ?? 1);
  displayedQuery = { keyword, sourceIds, precision, nextSourceIds: [] };
  if (nextPage === 1) searchResults.value = [];
  searchErrors.value = [];
  activeSearchCancellationPending.value = false;
  searchWasRun.value = true;
  searchBusy.value = true;
  searchHasMore.value = false;
  searchComplete.value = false;
  searchPage.value = nextPage;
  searchProgress.value = "正在启动搜索…";
  try {
    await initializeSearchState();
    if (currentRun !== run || run.stopRequested) {
      if (currentRun === run) searchProgress.value = "搜索已取消。";
      return;
    }
    const id = await search(sourceIds, keyword, nextPage, precision, batch => applyBatch(batch, run));
    if (currentRun !== run) { await releaseSearchResults(id); return; }
    searchHasMore.value = !run.stopRequested && displayedQuery.nextSourceIds.length > 0;
    searchComplete.value = !run.stopRequested;
    searchProgress.value = run.stopRequested ? "搜索已取消。" : "搜索完成";
  } catch (error) {
    if (currentRun !== run) return;
    searchProgress.value = "搜索失败。";
    notify(errorText(error), "error");
  } finally {
    run.finished = true;
    if (currentRun === run) {
      searchBusy.value = false;
      activeSearchCancellationPending.value = false;
    }
  }
}

export async function cancelActiveSearch(): Promise<void> {
  const run = currentRun;
  if (!run || run.finished) return;
  run.stopRequested = true;
  searchComplete.value = false;
  searchHasMore.value = false;
  activeSearchCancellationPending.value = true;
  searchProgress.value = "正在停止搜索…";
  // 编号未到达时，首次通知会再次调用；等待 search 返回后结束忙碌状态。
  if (!run.searchId || run.stopPromise) return run.stopPromise;
  run.stopPromise = stopSearch(run.searchId).catch(error => {
    if (currentRun !== run || run.finished) return;
    run.stopRequested = false;
    run.stopPromise = undefined;
    activeSearchCancellationPending.value = false;
    notify(`停止搜索失败：${errorText(error)}`, "error");
  });
  return run.stopPromise;
}

export function resetSearch(): void {
  currentRun = null;
  searchComplete.value = false;
  for (const id of retainedSearchIds) void releaseSearchResults(id).catch(console.error);
  retainedSearchIds.clear();
  displayedQuery = null;
  activeSearchCancellationPending.value = false;
  searchBusy.value = false;
  searchHasMore.value = false;
  searchWasRun.value = false;
  searchProgress.value = "";
  searchResults.value = [];
  searchErrors.value = [];
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
