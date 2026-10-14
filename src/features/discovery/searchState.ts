import { computed, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { startSearch, releaseSearchResults } from "../../api/search";
import { cancelTask } from "../../api/tasks";
import type { AppBootstrap, SearchBookResult, SearchBatch, SearchType } from "../../api/types";
import { bookSourceCandidates } from "../sources/sourcesStore";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";

export const searchType = ref<SearchType>("text");
export const selectedSourceIds = ref<string[]>([]);
export const searchKeyword = ref("");
export const searchPage = ref(1);
export const searchBusy = ref(false);
export const searchProgress = ref("");
export const searchResults = ref<SearchBookResult[]>([]);
export const searchResponseErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
export const searchResourceErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
export const searchWasRun = ref(false);
export const activeSearchTaskId = ref<string | null>(null);
const cancellingTaskId = ref<string | null>(null);
export const activeSearchCancellationPending = computed(() =>
  activeSearchTaskId.value !== null && cancellingTaskId.value === activeSearchTaskId.value);
export const searchSources = computed(() => bookSourceCandidates.value.filter(source =>
  source.searchType === searchType.value));

let retainedTaskId: string | null = null;
let generation = 0;
let starting = false;
let initialized = false;
let displayedQuery: { type: SearchType; keyword: string; sourceIds: string[] } | null = null;
let previousType: SearchType | null = null;
let previousIds: string[] = [];

watch([searchType, () => searchSources.value.map(source => source.id)], ([type, ids]) => {
  if (previousType !== type) {
    void cancelActiveSearch();
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

export async function initializeSearchState(): Promise<void> {
  if (initialized) return;
  initialized = true;
  await listen<AppBootstrap>("app-state-updated", resetSearch);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function applyBatch(batch: SearchBatch, expectedGeneration: number): void {
  if (generation !== expectedGeneration) return;
  if (activeSearchTaskId.value && batch.taskId !== activeSearchTaskId.value) return;
  if (batch.taskId) { activeSearchTaskId.value = batch.taskId; retainedTaskId = batch.taskId; }
  const results = Array.isArray(batch.results) ? batch.results : [];
  if (batch.incremental) {
    const merged = new Map(searchResults.value.map(result => [result.resultId, result]));
    for (const result of results) merged.set(result.resultId, result);
    searchResults.value = [...merged.values()];
  } else searchResults.value = results;
  searchResourceErrors.value = batch.errors ?? [];
  const status = batch.status ?? (batch.complete ? "completed" : "running");
  searchBusy.value = status === "queued" || status === "running";
  searchProgress.value = searchBusy.value
    ? `正在搜索（${batch.completedSources ?? 0}/${batch.totalSources ?? 0}）…`
    : status === "completed" ? `搜索完成 · ${searchResults.value.length} 项`
    : status === "cancelled" ? "搜索已取消。"
    : status === "paused" ? "搜索已暂停。" : "搜索失败。";
  if (batch.error) searchResponseErrors.value = [{ message: batch.error }];
  if (batch.complete || ["completed", "failed", "cancelled"].includes(status)) {
    activeSearchTaskId.value = null;
    cancellingTaskId.value = null;
  }
}

export async function runSearch(page?: number): Promise<void> {
  if (starting || shelfBatchRecoveryRequired.value) return;
  await initializeSearchState();
  const type = page === undefined ? searchType.value : displayedQuery?.type ?? searchType.value;
  const keyword = (page === undefined ? searchKeyword.value : displayedQuery?.keyword ?? searchKeyword.value).trim();
  const eligible = new Set(searchSources.value.map(source => source.id));
  const sourceIds = (page === undefined ? selectedSourceIds.value : displayedQuery?.sourceIds ?? selectedSourceIds.value)
    .filter(id => eligible.has(id));
  if (!keyword || !sourceIds.length) {
    notify(!keyword ? "先输入书名、作者或关键词。" : "请至少选择一个当前类型的已启用源。", "error");
    return;
  }
  const requestGeneration = ++generation;
  const nextPage = Math.max(1, page ?? 1);
  displayedQuery = { type, keyword, sourceIds };
  if (nextPage === 1) searchResults.value = [];
  searchResponseErrors.value = [];
  searchResourceErrors.value = [];
  activeSearchTaskId.value = null;
  cancellingTaskId.value = null;
  searchWasRun.value = true;
  searchBusy.value = true;
  searchPage.value = nextPage;
  searchProgress.value = "正在启动搜索…";
  starting = true;
  try {
    const response = await startSearch(type, sourceIds, keyword, nextPage,
      batch => applyBatch(batch, requestGeneration));
    if (generation !== requestGeneration) { await releaseSearchResults(response.taskId); return; }
    retainedTaskId = response.taskId;
    if (searchBusy.value) activeSearchTaskId.value = response.taskId;
  } catch (error) {
    if (generation !== requestGeneration) return;
    searchBusy.value = false;
    searchProgress.value = "搜索启动失败。";
    notify(errorText(error), "error");
  } finally { starting = false; }
}

export async function cancelActiveSearch(): Promise<void> {
  const id = activeSearchTaskId.value;
  if (!id || cancellingTaskId.value === id) return;
  cancellingTaskId.value = id;
  try { await cancelTask(id); }
  catch (error) {
    if (activeSearchTaskId.value === id) cancellingTaskId.value = null;
    notify(errorText(error), "error");
  }
}

export function resetSearch(): void {
  const taskId = activeSearchTaskId.value ?? retainedTaskId;
  retainedTaskId = null;
  if (taskId) void releaseSearchResults(taskId).catch(console.error);
  generation++;
  displayedQuery = null;
  activeSearchTaskId.value = null;
  cancellingTaskId.value = null;
  searchBusy.value = false;
  searchWasRun.value = false;
  searchProgress.value = "";
  searchResults.value = [];
  searchResponseErrors.value = [];
  searchResourceErrors.value = [];
  searchPage.value = 1;
}
