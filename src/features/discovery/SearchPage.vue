<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { addBook } from "../../api/books";
import { cancelTask } from "../../api/tasks";
import { startSearch } from "../../api/search";
import { readResource } from "../../api/resources";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryMessage, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { clearSearchSnapshot, publishSearchSnapshot, searchSnapshot } from "./searchSnapshotState";
import { loadBookSourceCandidates } from "../sources/sourceCandidates";
import type { AppBootstrap, BookResource, ResourceDescriptor, SearchBookResult, ShelfResource, SourceMetadata } from "../../api/types";
import SearchHistory from "./SearchHistory.vue";
import { useSearchHistory } from "./useSearchHistory";
import { useSearchResultActions } from "./useSearchResultActions";
import { useSearchWorkflow } from "./useSearchWorkflow";
import { discardBookDetailOpenRequest, requestBookDetailOpen, type BookDetailSearchResultActions } from "../books/bookDetailOpenRequest";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";

type SearchMatchMode = "contains" | "exact";
type SearchSortMode = "relevance" | "title" | "author";
type SearchContentType = "all" | "novel" | "comic" | "video" | "audio";
const contentTypes: Array<{ id: SearchContentType; label: string }> = [
  { id: "all", label: "全部" },
  { id: "novel", label: "小说" },
  { id: "comic", label: "漫画" },
  { id: "video", label: "视频" },
  { id: "audio", label: "音频" },
];

function returnFromSearch(): void {
  if (router.options.history.state.back) void router.back();
  else void router.replace({ name: routeNames.shelf });
}

function openSources(): void {
  void router.push({ name: routeNames.sources });
}

const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);

const restoreRevision = ref(0);
const pageDataLoading = ref(true);
const pageDataError = ref("");
const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const bookSourceCandidates = ref<SourceMetadata[]>([]);
let shelfReadRevision = 0;
let sourceCandidatesReadRevision = 0;
let bootstrapReadRevision = 0;
let disposed = false;
let lastSearchSnapshotId: string | null = null;
const eventUnlisteners: UnlistenFn[] = [];

async function refreshBookSourceCandidates(descriptor?: ResourceDescriptor): Promise<void> {
  const requestRevision = ++sourceCandidatesReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  const candidates = await loadBookSourceCandidates(descriptor);
  if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== sourceCandidatesReadRevision) return;
  bookSourceCandidates.value = candidates;
}

async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++shelfReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  const nextShelf = await readResource<ShelfResource>(descriptor);
  if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== shelfReadRevision) return;
  shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
  pageDataError.value = "";
}

async function applyBootstrap(snapshot: AppBootstrap): Promise<void> {
  const requestRevision = bootstrapReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  pageDataLoading.value = true;
  pageDataError.value = "";
  const historyRead = refreshSearchHistory(snapshot.searchHistory).then(() => {
    if (!disposed && currentRestoreRevision === restoreRevision.value) searchHistoryError.value = "";
  }).catch((error) => {
    if (!disposed && currentRestoreRevision === restoreRevision.value) searchHistoryError.value = errorText(error);
  });
  try {
    await Promise.all([refreshShelf(snapshot.shelf), refreshBookSourceCandidates(snapshot.sources), historyRead]);
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
  } catch (error) {
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
    pageDataError.value = errorText(error);
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision.value && requestRevision === bootstrapReadRevision) {
      pageDataLoading.value = false;
    }
  }
}

async function loadSearchPageData(): Promise<void> {
  const requestRevision = ++bootstrapReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  pageDataLoading.value = true;
  pageDataError.value = "";
  try {
    const snapshot = await appBootstrap();
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
    await applyBootstrap(snapshot);
  } catch (error) {
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
    pageDataError.value = errorText(error);
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision.value && requestRevision === bootstrapReadRevision) {
      pageDataLoading.value = false;
    }
  }
}

async function consumeSearchSnapshot(descriptor: ResourceDescriptor): Promise<void> {
  if (descriptor.resourceId === lastSearchSnapshotId) return;
  lastSearchSnapshotId = descriptor.resourceId;
  const currentRestoreRevision = restoreRevision.value;
  try {
    await searchWorkflow.consumeSearchSnapshot(descriptor);
    if (!disposed && currentRestoreRevision === restoreRevision.value) clearSearchSnapshot(descriptor.resourceId);
  } catch (error) {
    if (lastSearchSnapshotId === descriptor.resourceId) lastSearchSnapshotId = null;
    if (!disposed && currentRestoreRevision === restoreRevision.value) {
      clearSearchSnapshot(descriptor.resourceId);
      notify(errorText(error), "error");
    }
  }
}

async function setupEvents(): Promise<void> {
  function register(unlisten: UnlistenFn): void {
    if (disposed) unlisten();
    else eventUnlisteners.push(unlisten);
  }

  try {
    register(await listen<ResourceDescriptor>("search-results-updated", (event) => {
      publishSearchSnapshot(event.payload);
    }));
    register(await listen<ResourceDescriptor>("sources-updated", (event) => {
      void refreshBookSourceCandidates(event.payload).catch((error) => notify(errorText(error), "error"));
    }));
    register(await listen<ResourceDescriptor>("shelf-updated", (event) => {
      void refreshShelf(event.payload).catch((error) => notify(`书架刷新失败：${errorText(error)}`, "error"));
    }));
    register(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", (event) => {
      if (event.payload.kind === "searchHistory") {
        const currentRestoreRevision = restoreRevision.value;
        void refreshSearchHistory(event.payload.resource).then(() => {
          if (currentRestoreRevision === restoreRevision.value) searchHistoryError.value = "";
        }).catch((error) => {
          if (currentRestoreRevision === restoreRevision.value) searchHistoryError.value = errorText(error);
        });
      }
    }));
    register(await listen<AppBootstrap>("app-state-updated", (event) => {
      restoreRevision.value += 1;
      bootstrapReadRevision += 1;
      pageDataLoading.value = true;
      lastSearchSnapshotId = null;
      pageDataError.value = "";
      clearSearchSnapshot();
      void applyBootstrap(event.payload).catch((error) => notify(`恢复后搜索数据刷新失败：${errorText(error)}`, "error"));
    }));
  } catch (error) {
    console.debug("Search page event subscription unavailable:", errorText(error));
  }
}

onMounted(() => {
  void (async () => {
    await setupEvents();
    await loadSearchPageData();
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision.value += 1;
  bootstrapReadRevision += 1;
  shelfReadRevision += 1;
  sourceCandidatesReadRevision += 1;
  for (const unlisten of eventUnlisteners.splice(0)) unlisten();
});

const selectedSourceIds = ref<string[]>([]);
let sourceSelectionInitialized = false;
let previousCandidateIds: string[] = [];
watch(() => bookSourceCandidates.value.map((source) => source.id), (candidateIds) => {
  if (!sourceSelectionInitialized && candidateIds.length) {
    selectedSourceIds.value = [...candidateIds];
    sourceSelectionInitialized = true;
  } else {
    const eligible = new Set(candidateIds);
    const retained = selectedSourceIds.value.filter((id) => eligible.has(id));
    const retainedIds = new Set(retained);
    const newlyAvailable = candidateIds.filter((id) => !previousCandidateIds.includes(id) && !retainedIds.has(id));
    selectedSourceIds.value = [...retained, ...newlyAvailable];
  }
  previousCandidateIds = candidateIds;
}, { immediate: true });

const searchKeyword = ref("");
const searchPage = ref(1);
const searchBusy = ref(false);
const searchProgress = ref("");
const searchResponseErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
const searchResults = ref<SearchBookResult[]>([]);
const searchResourceErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
const searchWasRun = ref(false);
const searchResultActions = useSearchResultActions({
  searchResults,
  shelf,
  shelfBatchRecoveryRequired,
  shelfBatchRecoveryMessage,
  getRestoreRevision: () => restoreRevision.value,
  addBook,
  readBookResource: (descriptor) => readResource<BookResource>(descriptor),
  readShelfResource: (descriptor) => readResource<ShelfResource>(descriptor),
  errorText,
  notify,
});
const {
  selectedSearchResultIds,
  selectedSearchResults,
  searchResultBatchBusy,
  searchResultBatchSummary,
  searchResultAddErrors,
  busySearchResultKeys,
  searchResultSelectionKey,
  searchResultIsBusy,
  searchResultIsAdded,
  resetSearchResultState,
  pruneSelectionToResults,
  toggleSearchResultSelection,
  toggleVisibleSearchResultSelection,
  clearSearchResultSelection,
  addSelectedSearchResults,
} = searchResultActions;
const busySearchResultCount = computed(() => busySearchResultKeys.value.length);
const searchResultDetailActions: BookDetailSearchResultActions = {
  searchResultBatchBusy,
  searchResultIsBusy,
  addSearchResult: searchResultActions.addSearchResult,
};

const searchWorkflow = useSearchWorkflow({
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
  resetSearchResultState,
  pruneSelectionToResults,
  notify,
  errorText,
  getRestoreRevision: () => restoreRevision.value,
  startSearch,
  cancelTask,
  readResource,
});
const {
  activeSearchTaskId,
  activeSearchCancellationPending,
  runSearch,
  cancelActiveSearch,
} = searchWorkflow;
const searchHistory = useSearchHistory(errorText, () => restoreRevision.value);
const { searchHistoryEntries, searchHistoryBusy, searchHistoryError, refreshSearchHistory,
  removeSearchHistoryEntry, clearSearchHistoryEntries } = searchHistory;

watch(searchSnapshot, (descriptor) => {
  if (descriptor) void consumeSearchSnapshot(descriptor);
  else lastSearchSnapshotId = null;
}, { immediate: true });
watch(() => restoreRevision.value, () => {
  searchWorkflow.resetForRestore();
  searchKeyword.value = "";
  searchPage.value = 1;
  selectedSourceIds.value = bookSourceCandidates.value.map((source) => source.id);
  sourceSelectionInitialized = true;
  previousCandidateIds = bookSourceCandidates.value.map((source) => source.id);
});

function startNewSearch(): void {
  if (pageDataLoading.value || pageDataError.value || searchResultBatchBusy.value || shelfBatchRecoveryRequired.value) return;
  clearSearchSnapshot();
  clearSearchResultSelection();
  searchResultAddErrors.value = {};
  searchResultBatchSummary.value = "";
  void runSearch();
}

function updateKeyword(event: Event): void {
  searchKeyword.value = (event.target as HTMLInputElement).value;
}

function submitSearch(): void {
  startNewSearch();
}

function reuseSearchQuery(query: string): void {
  const value = query.trim();
  if (!value) return;
  searchKeyword.value = value;
  startNewSearch();
}

function setAllSearchSources(enabled: boolean): void {
  if (pageDataLoading.value || pageDataError.value || searchResultBatchBusy.value || shelfBatchRecoveryRequired.value) return;
  selectedSourceIds.value = enabled ? bookSourceCandidates.value.map((source) => source.id) : [];
}

function toggleSearchSource(sourceId: string): void {
  if (pageDataLoading.value || pageDataError.value || searchResultBatchBusy.value || shelfBatchRecoveryRequired.value
    || !bookSourceCandidates.value.some((source) => source.id === sourceId)) return;
  selectedSourceIds.value = selectedSourceIds.value.includes(sourceId)
    ? selectedSourceIds.value.filter((id) => id !== sourceId)
    : [...selectedSourceIds.value, sourceId];
}

async function openSearchResultDetails(result: SearchBookResult): Promise<void> {
  if (searchResultBatchBusy.value || shelfBatchRecoveryRequired.value) return;
  const requestId = requestBookDetailOpen({ kind: "search-result", result, actions: searchResultDetailActions });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

const titleFilter = ref("");
const authorFilter = ref("");
const matchMode = ref<SearchMatchMode>("contains");
const sourceFilter = ref("");
const sortMode = ref<SearchSortMode>("relevance");
const searchOptionsOpen = ref(false);
const resultFiltersOpen = ref(false);
const batchActionsOpen = ref(false);
const selectedContentType = ref<SearchContentType>("all");
const resultCollator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

function resultSourceLabel(result: SearchBookResult): string {
  const count = new Set((result.sources ?? [result]).map((source) => source.sourceId)).size;
  return count > 1 ? `${count} 个书源` : result.sourceName;
}

function sourceNameForId(sourceId: string): string {
  return bookSourceCandidates.value.find((source) => source.id === sourceId)?.name ?? sourceId;
}

function resultContentType(result: SearchBookResult): Exclude<SearchContentType, "all"> {
  // Only Rust-projected metadata; never classify using title or introduction.
  const declared = [result.mediaType, result.contentType, result.kind, result.type]
    .find((value) => typeof value === "string");
  if (typeof declared === "string") {
    if (/^(video|视频)$/i.test(declared)) return "video";
    if (/^(audio|音频)$/i.test(declared)) return "audio";
    if (/^(comic|manga|manhua|manhwa|漫画)$/i.test(declared)) return "comic";
    if (/^(novel|text|小说)$/i.test(declared)) return "novel";
  }
  const source = bookSourceCandidates.value.find((candidate) => candidate.id === result.sourceId);
  if (source?.mediaType === "video" || source?.mediaType === "audio") return source.mediaType;
  if (/漫画|comic|manga/i.test(source?.group ?? "")) return "comic";
  return "novel";
}

const selectedAllSources = computed(() => bookSourceCandidates.value.length > 0
  && selectedSourceIds.value.length === bookSourceCandidates.value.length
  && bookSourceCandidates.value.every((source) => selectedSourceIds.value.includes(source.id)));

const visibleSearchResults = computed(() => {
  const titleQuery = titleFilter.value.trim().toLocaleLowerCase("zh-CN");
  const authorQuery = authorFilter.value.trim().toLocaleLowerCase("zh-CN");
  const matches = (value: string, query: string) => {
    if (!query) return true;
    const normalizedValue = value.trim().toLocaleLowerCase("zh-CN");
    return matchMode.value === "exact" ? normalizedValue === query : normalizedValue.includes(query);
  };
  const results = searchResults.value.filter((result) => {
    return (selectedContentType.value === "all" || resultContentType(result) === selectedContentType.value)
      && (!sourceFilter.value || result.sourceId === sourceFilter.value
        || result.sources?.some((source) => source.sourceId === sourceFilter.value))
      && matches(result.title, titleQuery)
      && matches(result.author?.trim() || "作者未知", authorQuery);
  });
  if (sortMode.value === "relevance") return results;
  return results.map((result, index) => ({ result, index })).sort((left, right) => {
    const leftValue = sortMode.value === "author" ? left.result.author?.trim() ?? "" : left.result.title.trim();
    const rightValue = sortMode.value === "author" ? right.result.author?.trim() ?? "" : right.result.title.trim();
    if (sortMode.value === "author" && (!leftValue || !rightValue)) {
      if (!leftValue && rightValue) return 1;
      if (leftValue && !rightValue) return -1;
    }
    return resultCollator.compare(leftValue, rightValue) || left.index - right.index;
  }).map(({ result }) => result);
});

const resultSources = computed(() => {
  const names = new Map<string, string>();
  for (const result of searchResults.value) {
    names.set(result.sourceId, result.sourceName);
    for (const source of result.sources ?? []) names.set(source.sourceId, source.sourceName);
  }
  for (const sourceId of selectedSourceIds.value) {
    if (!names.has(sourceId)) names.set(sourceId, sourceNameForId(sourceId));
  }
  if (sourceFilter.value && !names.has(sourceFilter.value)) {
    names.set(sourceFilter.value, sourceNameForId(sourceFilter.value));
  }
  return [...names].sort((left, right) => resultCollator.compare(left[1], right[1]) || left[0].localeCompare(right[0]));
});

const hasResultFilters = computed(() => Boolean(titleFilter.value.trim()
  || authorFilter.value.trim() || sourceFilter.value
  || matchMode.value !== "contains" || sortMode.value !== "relevance"));
const addableVisibleResults = computed(() => visibleSearchResults.value.filter((result) => !searchResultIsAdded(result)));
const allVisibleAddableResultsSelected = computed(() => addableVisibleResults.value.length > 0
  && addableVisibleResults.value.every((result) => selectedSearchResultIds.value.includes(searchResultSelectionKey(result))));

function clearResultFilters(): void {
  titleFilter.value = "";
  authorFilter.value = "";
  matchMode.value = "contains";
  sourceFilter.value = "";
  sortMode.value = "relevance";
  selectedContentType.value = "all";
}
</script>

<template>
  <section class="search-view search-page-prototype">
    <header class="search-prototype-topbar">
      <BackButton label="返回" @click="returnFromSearch" />
      <form class="search-form search-prototype-top-input" @submit.prevent="submitSearch">
        <span class="search-large-icon"><PrototypeIcon name="search"/></span>
        <input
          :value="searchKeyword"
          autofocus
          placeholder="书名、作者、关键词或来源"
          :disabled="pageDataLoading || !!pageDataError || searchResultBatchBusy || shelfBatchRecoveryRequired"
          @input="updateKeyword"
        />
        <button v-if="searchKeyword" type="button" class="clear-search"
          :disabled="pageDataLoading || !!pageDataError || searchResultBatchBusy || shelfBatchRecoveryRequired"
          @click="searchKeyword = ''"><PrototypeIcon name="close"/></button>
      </form>
      <span class="search-prototype-topbar-spacer"></span>
    </header>
    <div class="search-prototype-layout">
      <nav class="search-mobile-categories">
        <button v-for="type in contentTypes" :key="type.id" type="button"
          :class="{ active: selectedContentType === type.id }"
          @click="selectedContentType = type.id">{{ type.label }}</button>
        <button class="search-mobile-filter-button" type="button"
          @click="searchOptionsOpen = !searchOptionsOpen"><PrototypeIcon name="tune"/></button>
      </nav>
      <aside class="source-scope-card" :class="{ 'mobile-open': searchOptionsOpen }">
        <section class="search-sidebar-section">
          <h3 class="search-prototype-sidebar-title">类型</h3>
          <div class="search-sidebar-type-list">
            <button v-for="type in contentTypes" :key="type.id" type="button"
              :class="{ active: selectedContentType === type.id }"
              @click="selectedContentType = type.id">{{ type.label }}</button>
          </div>
        </section>
        <section class="search-sidebar-section">
          <div class="search-sidebar-source-heading"><h3 class="search-prototype-sidebar-title">来源范围</h3>
            <button type="button" :disabled="pageDataLoading || !!pageDataError || searchResultBatchBusy || shelfBatchRecoveryRequired || !bookSourceCandidates.length"
              @click="setAllSearchSources(!selectedAllSources)">{{ selectedAllSources ? '取消全选' : '全选' }}</button></div>
          <span class="search-sidebar-count">已选 {{ selectedSourceIds.length }} / {{ bookSourceCandidates.length }}</span>
          <p v-if="pageDataLoading" class="search-sidebar-no-source">正在加载书源…</p>
          <div v-else-if="pageDataError" class="search-sidebar-no-source">搜索数据加载失败。
            <button type="button" @click="loadSearchPageData">重试</button>
          </div>
          <div v-else-if="bookSourceCandidates.length" class="source-chips">
            <label v-for="source in bookSourceCandidates" :key="source.id" class="source-chip"
              :class="{ checked: selectedSourceIds.includes(source.id) }">
              <input type="checkbox" :checked="selectedSourceIds.includes(source.id)"
                :disabled="pageDataLoading || !!pageDataError || searchResultBatchBusy || shelfBatchRecoveryRequired"
                @change="toggleSearchSource(source.id)"/>
              <span class="chip-label">{{ source.name }}</span>
            </label>
          </div>
          <div v-else class="search-sidebar-no-source">没有已启用的书源。
            <button type="button" @click="openSources">管理书源</button>
          </div>
        </section>
      </aside>
      <main class="search-prototype-results">
        <p v-if="pageDataLoading" class="search-list-progress">正在加载搜索数据…</p>
        <div v-if="pageDataError" class="search-errors">
          <p>搜索数据加载失败：{{ pageDataError }}</p>
          <button class="button secondary small" type="button" @click="loadSearchPageData">重试</button>
        </div>
        <SearchHistory v-if="!searchWasRun && (searchHistoryEntries.length || searchHistoryError)" :entries="searchHistoryEntries"
          :busy="searchHistoryBusy" :error="searchHistoryError"
          @use-query="reuseSearchQuery"
          @delete-query="removeSearchHistoryEntry"
          @clear-all="clearSearchHistoryEntries"/>
        <div class="results-section">
          <div class="search-list-heading">
            <div class="search-list-count">
              <strong>{{ selectedContentType === 'all' ? '全部结果' : contentTypes.find((item) => item.id === selectedContentType)?.label + '结果' }}</strong>
              <small>{{ visibleSearchResults.length }} 项</small>
            </div>
            <div class="search-list-actions">
              <span v-if="searchWasRun && searchProgress" class="search-list-progress">{{ searchProgress }}</span>
              <button v-if="searchBusy && activeSearchTaskId" type="button" class="search-list-utility"
                :disabled="activeSearchCancellationPending" @click="cancelActiveSearch">
                {{ activeSearchCancellationPending ? '取消中…' : '取消搜索' }}</button>
              <label v-if="searchWasRun && searchResults.length" class="search-list-sort">排序
                <select v-model="sortMode" :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired">
                  <option value="relevance">相关度</option><option value="title">书名</option><option value="author">作者</option>
                </select>
              </label>
              <button v-if="searchWasRun && searchResults.length" class="search-list-utility" type="button"
                @click="resultFiltersOpen = !resultFiltersOpen"><PrototypeIcon name="tune"/> {{ resultFiltersOpen ? '收起筛选' : '筛选' }}</button>
              <button v-if="searchWasRun && searchResults.length" class="search-list-utility" type="button"
                @click="batchActionsOpen = !batchActionsOpen">
                {{ batchActionsOpen ? '完成整理' : '批量加入' }}</button>
            </div>
          </div>
          <div v-if="!searchWasRun" class="search-start-inline">
            <PrototypeIcon name="search"/>
            <span>输入书名、作者或关键词，从所选 {{ selectedSourceIds.length }} 个书源搜索内容。</span>
            <button v-if="!bookSourceCandidates.length" type="button" @click="openSources">前往来源管理</button>
          </div>
          <template v-if="searchWasRun">
          <div v-if="searchBusy && !searchResults.length" class="search-loading"><span class="loader-ring"></span><strong>正在查找相关书籍</strong><p>正在从所选书源寻找匹配的书。</p></div>
      <template v-else-if="searchResults.length">
        <div v-if="resultFiltersOpen" class="source-toolbar advanced-search-toolbar">
          <label class="source-search"><PrototypeIcon name="search"/><input v-model="titleFilter" placeholder="输入书名" :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired" /></label>
          <label class="source-search"><PrototypeIcon name="search"/><input v-model="authorFilter" placeholder="输入作者" :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired" /></label>
          <select v-model="sourceFilter" :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired">
            <option value="">全部书源</option>
            <option v-for="[sourceId, name] in resultSources" :key="sourceId" :value="sourceId">{{ name }}</option>
          </select>
          <span class="search-status">匹配 {{ visibleSearchResults.length }} / {{ searchResults.length }} 本</span>
          <button v-if="hasResultFilters" class="text-button" type="button" @click="clearResultFilters">清空筛选</button>
        </div>
        <div v-if="batchActionsOpen || selectedSearchResults.length" class="source-toolbar search-batch-toolbar">
          <span class="search-status">已选 {{ selectedSearchResults.length }} 本 · 当前页</span>
          <button class="button secondary small" type="button" :disabled="!addableVisibleResults.length || searchResultBatchBusy || shelfBatchRecoveryRequired" @click="toggleVisibleSearchResultSelection(addableVisibleResults)">{{ allVisibleAddableResultsSelected ? '取消选中筛选结果' : '选中筛选结果' }}</button>
          <button class="button secondary small" type="button" :disabled="!selectedSearchResultIds.length || searchResultBatchBusy" @click="clearSearchResultSelection">清空选择</button>
          <button class="button primary small" type="button" :disabled="!selectedSearchResults.length || busySearchResultCount > 0 || searchResultBatchBusy || shelfBatchRecoveryRequired" @click="addSelectedSearchResults">{{ searchResultBatchBusy ? '正在加入…' : `加入书架 · ${selectedSearchResults.length}` }}</button>
        </div>
        <p v-if="searchResultBatchSummary" class="search-status">{{ searchResultBatchSummary }}</p>
        <div v-if="visibleSearchResults.length" class="search-results-grid search-prototype-result-list">
          <article v-for="result in visibleSearchResults" :key="searchResultSelectionKey(result)"
            class="result-card">
            <button class="result-cover" type="button"
              :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired || searchResultIsBusy(result)"
              @click="openSearchResultDetails(result)">
              <img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" decoding="async"/>
              <span v-else class="search-abstract-cover" :class="`search-tone-${result.title.length % 5}`"></span>
            </button>
            <div class="result-details">
              <button class="result-title" type="button"
                :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired || searchResultIsBusy(result)"
                @click="openSearchResultDetails(result)">{{ result.title }}</button>
              <p class="result-author">{{ result.author || '作者未知' }} · {{ contentTypes.find((item) => item.id === resultContentType(result))?.label }}</p>
              <p v-if="result.intro" class="search-result-intro">{{ String(result.intro) }}</p>
              <p v-else-if="result.latestChapter" class="search-result-intro">{{ result.latestChapter }}</p>
              <p v-if="searchResultAddErrors[searchResultSelectionKey(result)]" class="result-latest">
                加入失败：{{ searchResultAddErrors[searchResultSelectionKey(result)] }}</p>
            </div>
            <div class="search-result-side">
              <small class="source-label">{{ resultSourceLabel(result) }}</small>
              <button class="search-result-open" type="button"
                :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired || searchResultIsBusy(result)"
                @click="openSearchResultDetails(result)">打开</button>
              <button v-if="batchActionsOpen || selectedSearchResultIds.includes(searchResultSelectionKey(result))"
                class="result-select-action" type="button"
                :disabled="searchResultBatchBusy || shelfBatchRecoveryRequired || searchResultIsBusy(result) || searchResultIsAdded(result)"
                @click="toggleSearchResultSelection(result)">
                {{ searchResultIsAdded(result) ? '已在书架' : selectedSearchResultIds.includes(searchResultSelectionKey(result)) ? '已选中 ✓' : '选择加入 +' }}
              </button>
            </div>
          </article>
        </div>
        <div v-if="!visibleSearchResults.length" class="empty-card results-empty">
          <div class="empty-icon"><PrototypeIcon name="search"/></div><h3>本页没有符合条件的书</h3>
          <p>调整标题或作者筛选词，或继续浏览其他搜索页面。</p>
          <button class="button secondary small" type="button" @click="clearResultFilters">清除筛选</button>
        </div>
      </template>
      <div v-else-if="!searchBusy" class="search-empty-inline">
        没有找到匹配内容。可以调整关键词或更换参与搜索的来源。
      </div>
      <div v-if="searchResponseErrors.length || searchResourceErrors.length" class="search-errors">
        <strong>{{ selectedSourceIds.length === 1 ? '这个书源没有完成搜索' : '部分书源没有完成搜索' }}</strong>
        <ul><li v-for="(error, index) in [...searchResponseErrors, ...searchResourceErrors]" :key="`${error.sourceId}-${index}`"><b>{{ error.sourceId ? sourceNameForId(error.sourceId) : '书源' }}</b>：{{ error.message }}</li></ul>
      </div>
      <div v-if="searchResults.length" class="search-pagination">
        <button class="button secondary small" :disabled="pageDataLoading || !!pageDataError || searchPage <= 1 || searchBusy || searchResultBatchBusy || shelfBatchRecoveryRequired" @click="runSearch(searchPage - 1)">← 上一页</button>
        <span>第 {{ searchPage }} 页</span>
        <button class="button secondary small" :disabled="pageDataLoading || !!pageDataError || searchBusy || searchResultBatchBusy || shelfBatchRecoveryRequired" @click="runSearch(searchPage + 1)">下一页 →</button>
      </div>
      </template>
      </div>
      </main>
    </div>
  </section>
</template>

<style scoped>
.search-page-prototype {
  width: 100%;
  padding-bottom: 20px;
}
.search-prototype-topbar {
  position: sticky;
  top: 0;
  z-index: 3;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 54px;
  padding: 0 12px;
  border-bottom: 1px solid var(--app-line);
  background: var(--app-background);
}
.search-prototype-topbar-spacer {
  flex: none;
  width: 44px;
  height: 44px;
}
.search-prototype-top-input {
  display: flex;
  align-items: center;
  flex: 0 1 680px;
  gap: 8px;
  min-width: 0;
  height: 44px;
  margin: 0;
  padding: 0 9px 0 12px;
  border: 1px solid var(--app-line);
  border-radius: 12px;
  background: var(--app-surface);
  box-shadow: none;
  color: var(--app-text);
}
.search-prototype-top-input input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--app-text);
  font-size: 14px;
}
.search-prototype-top-input input::placeholder {
  color: var(--app-muted);
}
.search-prototype-top-input .clear-search {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  flex: none;
  border: 0;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  cursor: pointer;
}
.search-prototype-top-input .clear-search:hover {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.search-prototype-layout {
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
  gap: 20px;
  width: min(1120px, calc(100% - 32px));
  margin: 12px auto;
}
.search-prototype-layout > .source-scope-card {
  position: sticky;
  top: 66px;
  display: grid;
  align-content: start;
  gap: 12px;
  max-height: calc(100dvh - 78px);
  overflow: auto;
}
.search-sidebar-section, .search-sidebar-type-list, .search-prototype-layout .source-chips {
  display: grid;
  gap: 5px;
}
.search-prototype-sidebar-title {
  margin: 0;
  font-size: 12px;
}
.search-sidebar-type-list button, .search-sidebar-source-heading button,
.search-sidebar-no-source button {
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
}
.search-sidebar-type-list button.active, .search-prototype-layout .source-chip:hover {
  background: var(--app-accent-soft);
}
.search-sidebar-source-heading, .search-list-heading, .search-list-count,
.search-list-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.search-sidebar-source-heading, .search-list-heading {
  justify-content: space-between;
}
.search-sidebar-count, .search-list-count small, .search-list-progress,
.search-list-sort, .search-status {
  color: var(--app-muted);
  font-size: 11px;
}
.search-prototype-layout .source-chip {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 28px;
}
.search-prototype-layout .source-chip input {
  position: static;
  width: 13px;
  height: 13px;
  opacity: 1;
  accent-color: var(--app-accent);
}
.search-prototype-layout .chip-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.search-sidebar-no-source {
  font-size: 12px;
}
.search-mobile-categories {
  display: none;
}
.search-mobile-categories button {
  display: grid;
  place-items: center;
  min-height: 34px;
  flex: none;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  color: var(--app-muted);
  background: transparent;
  font-size: 12px;
  white-space: nowrap;
  cursor: pointer;
}
.search-mobile-categories button.active {
  color: #fff;
  background: #202126;
  font-weight: 700;
}
.search-mobile-categories button:hover:not(.active) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.search-mobile-filter-button {
  width: 36px;
  height: 36px;
  padding: 0 !important;
  border-radius: 9px !important;
}
.search-prototype-results {
  min-width: 0;
}
.search-list-heading {
  min-height: 42px;
  border-bottom: 1px solid var(--app-line);
}
.search-list-count strong {
  color: var(--app-text);
  font-size: 14px;
}
.search-list-actions {
  flex-wrap: wrap;
  justify-content: flex-end;
}
.search-list-sort select {
  max-width: 100px;
  border: 0;
  background: transparent;
}
.search-list-utility {
  min-height: 32px;
  padding: 0 9px;
  border: 0;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  font-size: 12px;
  cursor: pointer;
}
.search-list-utility:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.search-start-inline, .search-empty-inline {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 80px;
  padding: 12px;
  color: var(--app-muted);
  font-size: 13px;
  line-height: 1.5;
}
.search-start-inline :deep(.prototype-icon) {
  width: 18px;
  height: 18px;
  flex: none;
}
.search-start-inline button {
  border: 0;
  background: transparent;
  color: var(--app-accent);
  cursor: pointer;
}
.search-prototype-result-list {
  display: grid;
}
.search-prototype-result-list .result-card {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  min-height: 70px;
  padding: 8px 0;
  border: 0;
  border-bottom: 1px solid var(--app-line);
  border-radius: 0;
  background: transparent;
}
.search-prototype-result-list .result-cover {
  width: 48px;
  height: 60px;
  overflow: hidden;
  padding: 0;
  border: 0;
}
.search-prototype-result-list .result-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.search-abstract-cover {
  display: block;
  width: 100%;
  height: 100%;
  background: var(--app-accent-soft);
}
.search-prototype-result-list .result-details {
  display: grid;
  gap: 3px;
  min-width: 0;
}
.search-prototype-result-list .result-title {
  color: var(--app-text);
  font-size: 14px;
  font-weight: 700;
  line-height: 1.4;
  text-align: left;
  cursor: pointer;
}
.search-prototype-result-list .result-title:hover:not(:disabled) {
  color: var(--app-accent-ink);
}
.search-prototype-result-list .result-title:disabled {
  color: var(--app-muted);
  cursor: default;
}
.search-prototype-result-list .result-title, .search-prototype-result-list .result-author,
.search-prototype-result-list .search-result-intro, .search-prototype-result-list .source-label {
  overflow: hidden;
  margin: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.search-prototype-result-list .search-result-side {
  display: flex;
  align-items: center;
  gap: 6px;
}
.search-prototype-results .advanced-search-toolbar, .search-prototype-results .search-batch-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 10px 0;
}
.search-loading, .empty-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
}
.search-loading {
  min-height: 120px;
}
.empty-card {
  min-height: 170px;
  padding: 16px;
}
.search-errors {
  margin-top: 12px;
  padding: 12px;
  border: 1px solid var(--app-line);
}
.search-errors ul {
  margin: 8px 0 0;
  padding-left: 18px;
}
.search-pagination {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 8px;
  margin-top: 14px;
}
@media (max-width: 860px) {
  .search-prototype-layout {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    margin: 0;
  }
  .search-mobile-categories {
    display: flex;
    align-items: center;
    gap: 4px;
    overflow-x: auto;
    padding: 8px 12px;
  }
  .search-mobile-filter-button {
    margin-left: auto;
  }
  .search-prototype-layout > .source-scope-card {
    position: static;
    display: none;
    max-height: 50dvh;
    padding: 12px;
  }
  .search-prototype-layout > .source-scope-card.mobile-open {
    display: grid;
  }
  .search-prototype-results {
    box-sizing: border-box;
    padding: 0 12px 24px;
  }
  .search-prototype-result-list .result-card {
    grid-template-columns: 42px minmax(0, 1fr) auto;
    gap: 8px;
  }
  .search-prototype-result-list .result-cover {
    width: 42px;
    height: 52px;
  }
  .search-prototype-result-list .search-result-side {
    align-items: flex-end;
    flex-direction: column;
  }
}
@media (max-width: 530px) {
  .search-prototype-topbar-spacer {
    display: none;
  }
  .search-prototype-topbar {
    justify-content: stretch;
  }
  .search-prototype-top-input {
    flex: 1;
  }
}
</style>
