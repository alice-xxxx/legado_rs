<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import type { SearchBookResult, SearchType } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import SearchHistory from "./SearchHistory.vue";
import { useSearchHistory } from "./useSearchHistory";
import { SEARCH_RESULT_ROW_HEIGHT, useSearchResultWindow } from "./useSearchResultWindow";
import {
  searchType as selectedContentType, searchSources, selectedSourceIds, searchKeyword,
  searchBusy, searchProgress, searchResponseErrors, searchResults,
  searchResourceErrors, searchWasRun, searchHasMore, searchComplete, loadMoreSearchResults,
  activeSearchCancellationPending, runSearch, cancelActiveSearch,
  precisionSearch,
} from "./searchState";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";

defineOptions({ name: "SearchPage" });

type SearchMatchMode = "contains" | "exact";
type SearchSortMode = "relevance" | "title" | "author";
type SearchContentType = SearchType;
const contentTypes: Array<{ id: SearchContentType; label: string }> = [
  { id: "text", label: "文本" },
  { id: "image", label: "图片" },
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

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}


const { searchHistoryEntries, removeSearchHistoryEntry, clearSearchHistoryEntries } = useSearchHistory();

function startNewSearch(): void {
  if (shelfBatchRecoveryRequired.value) return;
  scrollResultsToStart();
  clearResultFilters();
  resultFiltersOpen.value = false;
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
  if (shelfBatchRecoveryRequired.value) return;
  selectedSourceIds.value = enabled ? searchSources.value.map((source) => source.id) : [];
}

function toggleSearchSource(sourceId: string): void {
  if (shelfBatchRecoveryRequired.value
    || !searchSources.value.some((source) => source.id === sourceId)) return;
  selectedSourceIds.value = selectedSourceIds.value.includes(sourceId)
    ? selectedSourceIds.value.filter((id) => id !== sourceId)
    : [...selectedSourceIds.value, sourceId];
}

async function openSearchResultDetails(result: SearchBookResult): Promise<void> {
  if (shelfBatchRecoveryRequired.value || openingResultDetails.value
    || router.currentRoute.value.name !== routeNames.search) return;
  openingResultDetails.value = true;
  rememberResultPosition();
  const requestId = requestBookDetailOpen({ kind: "search-result", result });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) { discardBookDetailOpenRequest(requestId); forgetResultPosition(); }
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    forgetResultPosition();
    notify(errorText(error), "error");
  } finally {
    openingResultDetails.value = false;
  }
}

const titleFilter = ref("");
const openingResultDetails = ref(false);
const authorFilter = ref("");
const matchMode = ref<SearchMatchMode>("contains");
const sourceFilter = ref("");
const sortMode = ref<SearchSortMode>("relevance");
const searchOptionsOpen = ref(false);
const resultFiltersOpen = ref(false);
const resultCollator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });
const loadMoreMarker = ref<HTMLElement | null>(null);
let touchY: number | null = null;
let loadMoreFrame = 0;

function checkLoadMore(): void {
  loadMoreFrame = 0;
  if (router.currentRoute.value.name !== routeNames.search || searchBusy.value || !searchHasMore.value) return;
  const marker = loadMoreMarker.value;
  if (marker && marker.getBoundingClientRect().top <= window.innerHeight + 40) {
    void loadMoreSearchResults();
  }
}
function scheduleLoadMore(): void {
  if (!loadMoreFrame) loadMoreFrame = requestAnimationFrame(checkLoadMore);
}
function onSearchWheel(event: WheelEvent): void {
  if (event.deltaY > 0) scheduleLoadMore();
}
function onSearchTouchStart(event: TouchEvent): void {
  touchY = event.touches[0]?.clientY ?? null;
}
function onSearchTouchMove(event: TouchEvent): void {
  const nextY = event.touches[0]?.clientY ?? null;
  if (nextY !== null && touchY !== null && nextY < touchY) scheduleLoadMore();
  touchY = nextY;
}
function onSearchKeyDown(event: KeyboardEvent): void {
  if (!(event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement)
    && ["ArrowDown", "PageDown", "End"].includes(event.key)) scheduleLoadMore();
}
onBeforeUnmount(() => cancelAnimationFrame(loadMoreFrame));

function resultSourceLabel(result: SearchBookResult): string {
  const count = new Set((result.sources ?? [result]).map((source) => source.sourceId)).size;
  return count > 1 ? `${count} 个源` : result.sourceName;
}

function sourceNameForId(sourceId: string): string {
  return sourcesById.value.get(sourceId)?.name ?? sourceId;
}

const sourcesById = computed(() => new Map(searchSources.value.map(source => [source.id, source])));
const selectedSourceSet = computed(() => new Set(selectedSourceIds.value));

function resultContentType(result: SearchBookResult): SearchContentType {
  const source = sourcesById.value.get(result.sourceId);
  const type = bookContentType(result) ?? bookContentType(source) ?? "text";
  return type as SearchContentType;
}

const selectedAllSources = computed(() => searchSources.value.length > 0
  && selectedSourceIds.value.length === searchSources.value.length
  && searchSources.value.every((source) => selectedSourceSet.value.has(source.id)));

const visibleSearchResults = computed(() => {
  const titleQuery = titleFilter.value.trim().toLocaleLowerCase("zh-CN");
  const authorQuery = authorFilter.value.trim().toLocaleLowerCase("zh-CN");
  const matches = (value: string, query: string) => {
    if (!query) return true;
    const normalizedValue = value.trim().toLocaleLowerCase("zh-CN");
    return matchMode.value === "exact" ? normalizedValue === query : normalizedValue.includes(query);
  };
  const results = searchResults.value.filter((result) => {
    return (resultContentType(result) === selectedContentType.value)
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

const resultListElement = ref<HTMLElement | null>(null);
const { range: resultWindow, renderedResults, viewportWidth,
  rememberPosition: rememberResultPosition, forgetPosition: forgetResultPosition, scrollToStart: scrollResultsToStart,
} = useSearchResultWindow(visibleSearchResults, resultListElement);
watch([titleFilter, authorFilter, sourceFilter, matchMode, sortMode], scrollResultsToStart, { flush: "post" });

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
function clearResultFilters(): void {
  titleFilter.value = "";
  authorFilter.value = "";
  matchMode.value = "contains";
  sourceFilter.value = "";
  sortMode.value = "relevance";
}
watch(selectedContentType, () => { clearResultFilters(); resultFiltersOpen.value = false; });
</script>

<template>
  <section class="search-view search-page-prototype">
    <header class="search-prototype-topbar">
      <div class="search-topbar-content">
        <BackButton class="search-back-button" label="返回" @click="returnFromSearch" />
        <form class="search-form search-prototype-top-input" @submit.prevent="submitSearch">
          <button class="search-submit" type="submit" aria-label="搜索" title="搜索"
            :disabled="!searchKeyword.trim() || shelfBatchRecoveryRequired">
            <PrototypeIcon name="search" />
          </button>
          <input :value="searchKeyword" autofocus aria-label="搜索关键词" enterkeyhint="search" placeholder="搜索标题、作者或关键词"
            :disabled="shelfBatchRecoveryRequired" @input="updateKeyword" />
          <button v-if="searchKeyword" type="button" class="clear-search" aria-label="清空搜索" title="清空搜索"
            :disabled="shelfBatchRecoveryRequired" @click="searchKeyword = ''">
            <PrototypeIcon name="close" />
          </button>
        </form>
      </div>
    </header>
    <div class="search-prototype-layout">
      <nav class="search-type-tabs" aria-label="搜索类型">
        <div class="search-type-options">
          <button v-for="type in contentTypes" :key="type.id" type="button"
            :class="{ active: selectedContentType === type.id }" :aria-pressed="selectedContentType === type.id"
            :disabled="shelfBatchRecoveryRequired" @click="selectedContentType = type.id">{{ type.label }}</button>
        </div>
        <label class="search-precision" title="只显示书名或作者包含搜索关键词的结果">
          <input v-model="precisionSearch" type="checkbox"
            :disabled="activeSearchCancellationPending || shelfBatchRecoveryRequired"
            @change="searchWasRun && startNewSearch()" />
          精准搜索
        </label>
        <button class="search-scope-toggle" type="button" :aria-expanded="searchOptionsOpen"
          @click="searchOptionsOpen = !searchOptionsOpen">源</button>
      </nav>
      <aside class="source-scope-card" :class="{ 'mobile-open': searchOptionsOpen }">
        <section class="search-sidebar-section">
          <div class="search-sidebar-source-heading">
            <h3 class="search-prototype-sidebar-title">来源范围</h3>
            <button type="button" :disabled="shelfBatchRecoveryRequired || !searchSources.length"
              @click="setAllSearchSources(!selectedAllSources)">{{ selectedAllSources ? '取消全选' : '全选' }}</button>
          </div>
          <span class="search-sidebar-count">已选 {{ selectedSourceIds.length }} / {{ searchSources.length }}</span>
          <div v-if="searchSources.length && (viewportWidth > 860 || searchOptionsOpen)" class="source-chips">
            <label v-for="source in searchSources" :key="source.id" class="source-chip"
              :class="{ checked: selectedSourceSet.has(source.id) }">
              <input type="checkbox" :checked="selectedSourceSet.has(source.id)"
                :disabled="shelfBatchRecoveryRequired" @change="toggleSearchSource(source.id)" />
              <span class="chip-label">{{ source.name }}</span>
              <small class="search-source-type">{{contentTypes.find(type => type.id === source.searchType)?.label
                }}</small>
            </label>
          </div>
          <div v-else-if="!searchSources.length" class="search-sidebar-no-source">没有已启用的源。
            <button type="button" @click="openSources">管理源</button>
          </div>
        </section>
      </aside>
      <main class="search-prototype-results" @wheel.passive="onSearchWheel" @touchstart.passive="onSearchTouchStart"
        @touchmove.passive="onSearchTouchMove" @keydown="onSearchKeyDown">
        <SearchHistory v-if="!searchWasRun && searchHistoryEntries.length" :entries="searchHistoryEntries"
          @use-query="reuseSearchQuery" @delete-query="removeSearchHistoryEntry"
          @clear-all="clearSearchHistoryEntries" />
        <div class="results-section">
          <div v-if="searchWasRun" class="search-list-heading">
            <div class="search-list-count">
              <small>{{ visibleSearchResults.length }} 项</small>
            </div>
            <div class="search-list-actions">
              <span v-if="searchWasRun && searchProgress" class="search-list-progress">{{ searchProgress }}</span>
              <button v-if="searchBusy || activeSearchCancellationPending" type="button" class="search-list-utility"
                :disabled="activeSearchCancellationPending" @click="cancelActiveSearch">
                {{ activeSearchCancellationPending ? '取消中…' : '取消搜索' }}</button>
              <label v-if="searchWasRun && searchResults.length" class="search-list-sort">排序
                <select v-model="sortMode" :disabled="shelfBatchRecoveryRequired">
                  <option value="relevance">相关度</option>
                  <option value="title">书名</option>
                  <option value="author">作者</option>
                </select>
              </label>
              <button v-if="searchWasRun && searchResults.length" class="search-list-utility" type="button"
                @click="resultFiltersOpen = !resultFiltersOpen">
                <PrototypeIcon name="tune" /> {{ resultFiltersOpen ? '收起筛选' : '筛选' }}
              </button>
            </div>
          </div>
          <template v-if="searchWasRun">
            <div v-if="searchBusy && !searchResults.length" class="search-loading"><span
                class="loader-ring"></span><strong>正在搜索</strong></div>
            <template v-else-if="searchResults.length">
              <div v-if="resultFiltersOpen" class="source-toolbar advanced-search-toolbar">
                <label class="source-search">
                  <PrototypeIcon name="search" /><input v-model="titleFilter" placeholder="输入书名"
                    :disabled="shelfBatchRecoveryRequired" />
                </label>
                <label class="source-search">
                  <PrototypeIcon name="search" /><input v-model="authorFilter" placeholder="输入作者"
                    :disabled="shelfBatchRecoveryRequired" />
                </label>
                <select v-model="sourceFilter" :disabled="shelfBatchRecoveryRequired">
                  <option value="">全部源</option>
                  <option v-for="[sourceId, name] in resultSources" :key="sourceId" :value="sourceId">{{ name }}
                  </option>
                </select>
                <span class="search-status">匹配 {{ visibleSearchResults.length }} / {{ searchResults.length }} 本</span>
                <button v-if="hasResultFilters" class="text-button" type="button"
                  @click="clearResultFilters">清空筛选</button>
              </div>
              <div v-if="visibleSearchResults.length" ref="resultListElement" class="search-results-grid search-prototype-result-list"
                role="list" :style="{ '--search-result-row-height': `${SEARCH_RESULT_ROW_HEIGHT}px` }">
                <div aria-hidden="true" :style="{ height: `${resultWindow.top}px` }"></div>
                <article v-for="(result, index) in renderedResults" :key="result.resultId" class="result-card"
                  role="listitem" :aria-posinset="resultWindow.start + index + 1" :aria-setsize="visibleSearchResults.length">
                  <button class="result-cover" type="button" :disabled="shelfBatchRecoveryRequired || openingResultDetails"
                    @click="openSearchResultDetails(result)">
                    <img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" decoding="async" />
                    <span v-else class="search-abstract-cover" :class="`search-tone-${result.title.length % 5}`"></span>
                  </button>
                  <div class="result-details">
                    <button class="result-title" type="button" :disabled="shelfBatchRecoveryRequired || openingResultDetails"
                      @click="openSearchResultDetails(result)">{{ result.title }}</button>
                    <p class="result-author">{{ result.author || '作者未知' }} · {{contentTypes.find((item) => item.id ===
                      resultContentType(result))?.label }}</p>
                    <p v-if="result.intro" class="search-result-intro">{{ String(result.intro) }}</p>
                    <p v-else-if="result.latestChapter" class="search-result-intro">{{ result.latestChapter }}</p>
                  </div>
                  <div class="search-result-side">
                    <small class="source-label">{{ resultSourceLabel(result) }}</small>
                    <button class="search-result-open" type="button" :disabled="shelfBatchRecoveryRequired || openingResultDetails"
                      @click="openSearchResultDetails(result)">打开</button>
                  </div>
                </article>
                <div aria-hidden="true" :style="{ height: `${resultWindow.bottom}px` }"></div>
              </div>
              <div v-if="!visibleSearchResults.length" class="empty-card results-empty">
                <div class="empty-icon">
                  <PrototypeIcon name="search" />
                </div>
                <h3>本页没有符合条件的书</h3>
                <p>调整标题或作者筛选词，或继续浏览其他搜索页面。</p>
                <button class="button secondary small" type="button" @click="clearResultFilters">清除筛选</button>
              </div>
            </template>
            <div v-else-if="!searchBusy" class="search-empty-inline">
              没有找到匹配内容。可以调整关键词或更换参与搜索的来源。
              <button v-if="precisionSearch && !searchResponseErrors.length" class="text-button" type="button"
                :disabled="shelfBatchRecoveryRequired"
                @click="precisionSearch = false; startNewSearch()">关闭精准搜索，查看全部结果</button>
            </div>
            <div v-if="searchResponseErrors.length || searchResourceErrors.length" class="search-errors">
              <strong>{{ selectedSourceIds.length === 1 ? '这个源没有完成搜索' : '部分源没有完成搜索' }}</strong>
              <ul>
                <li v-for="(error, index) in [...searchResponseErrors, ...searchResourceErrors]"
                  :key="`${error.sourceId}-${index}`"><b>{{ error.sourceId ? sourceNameForId(error.sourceId) : '源'
                    }}</b>：{{
                  error.message }}</li>
              </ul>
            </div>
            <div ref="loadMoreMarker" class="search-load-more" aria-live="polite">
              <span v-if="searchBusy && searchResults.length">正在加载更多…</span>
              <span v-else-if="searchHasMore">继续向下滑动，加载更多</span>
              <span v-else-if="searchComplete && searchResults.length">没有更多结果</span>
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
  min-width: 0;
  max-width: 100%;
  overflow-x: clip;
  padding-bottom: 20px;
}

.search-prototype-topbar {
  position: sticky;
  top: 0;
  z-index: 3;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: calc(56px + var(--safe-top));
  padding: calc(8px + var(--safe-top)) 16px 8px;
  border-bottom: 1px solid var(--app-line);
  background: var(--app-background);
}

.search-topbar-content {
  display: flex;
  align-items: center;
  gap: 8px;
  width: min(760px, 100%);
  min-width: 0;
}

.search-back-button {
  width: 36px;
  min-width: 36px;
  height: 40px;
  min-height: 40px;
  border-radius: 10px;
  color: var(--app-text);
}

.search-back-button:hover {
  background: var(--app-panel-soft);
}

.search-prototype-top-input {
  display: flex;
  align-items: center;
  flex: 1;
  gap: 4px;
  min-width: 0;
  height: 40px;
  margin: 0;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: var(--app-panel-soft);
  box-shadow: none;
  color: var(--app-text);
}

.search-prototype-top-input:focus-within {
  border-color: color-mix(in srgb, var(--app-accent) 35%, var(--app-line));
  background: var(--app-surface);
}

.search-submit {
  display: grid;
  place-items: center;
  flex: none;
  width: 32px;
  height: 36px;
  padding: 0;
  border: 0;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  cursor: pointer;
}

.search-submit:hover:not(:disabled) {
  color: var(--app-accent-ink);
}

.search-submit:disabled {
  cursor: default;
}

.search-submit :deep(svg),
.clear-search :deep(svg) {
  width: 18px;
  height: 18px;
}

.search-back-button:focus-visible,
.search-submit:focus-visible,
.clear-search:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}

.search-prototype-top-input input {
  flex: 1;
  min-width: 0;
  padding: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--app-text);
  font-size: var(--ui-font-body);
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
  color: var(--app-text);
  background: var(--app-panel-soft);
}

.search-prototype-layout {
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
  gap: 20px;
  width: min(1120px, calc(100% - 32px));
  margin: 12px auto;
}

.search-prototype-layout>.source-scope-card {
  position: sticky;
  top: calc(66px + var(--safe-top));
  display: grid;
  align-content: start;
  gap: 12px;
  max-height: calc(100dvh - 78px);
  overflow: auto;
}

.search-sidebar-section,
.search-prototype-layout .source-chips {
  display: grid;
  gap: 5px;
}

.search-prototype-sidebar-title {
  margin: 0;
  font-size: var(--ui-font-body);
}

.search-sidebar-source-heading button,
.search-sidebar-no-source button {
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.search-sidebar-source-heading,
.search-list-heading,
.search-list-count,
.search-list-actions,
.search-precision {
  display: flex;
  align-items: center;
  gap: 8px;
}

.search-sidebar-source-heading,
.search-list-heading {
  justify-content: space-between;
}

.search-sidebar-count,
.search-list-count small,
.search-list-progress,
.search-list-sort,
.search-status {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-precision {
  cursor: pointer;
  font-size: var(--ui-font-caption);
  white-space: nowrap;
}

.search-precision input {
  accent-color: var(--app-accent);
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
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-source-type {
  flex: none;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-load-more {
  padding: 14px 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  text-align: center;
}

.search-sidebar-no-source {
  font-size: var(--ui-font-caption);
}

.search-type-tabs {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.search-type-options {
  display: flex;
  flex: 1;
  gap: 4px;
  min-width: 0;
  overflow-x: auto;
}

.search-type-options button,
.search-scope-toggle {
  display: grid;
  place-items: center;
  min-height: 34px;
  flex: none;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  color: var(--app-muted);
  background: transparent;
  font-size: var(--ui-font-body);
  white-space: nowrap;
  cursor: pointer;
}

.search-type-options button.active {
  color: var(--app-on-selected);
  background: var(--app-selected);
  font-weight: 700;
}

.search-scope-toggle {
  display: none;
}

.search-prototype-results {
  min-width: 0;
  max-width: 100%;
}

.search-list-heading {
  min-height: 42px;
  border-bottom: 1px solid var(--app-line);
}

.search-list-actions {
  min-width: 0;
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
  font-size: var(--ui-font-caption);
  cursor: pointer;
}

.search-empty-inline {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 80px;
  padding: 12px;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  line-height: 1.5;
}

.search-prototype-result-list {
  display: block;
  overflow-anchor: none;
}

.search-prototype-result-list .result-card {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  height: var(--search-result-row-height);
  min-height: var(--search-result-row-height);
  box-sizing: border-box;
  contain: layout paint;
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
  grid-template-columns: minmax(0, 1fr);
  gap: 3px;
  min-width: 0;
}

.search-prototype-result-list .result-title {
  min-width: 0;
  width: 100%;
  color: var(--app-text);
  font-size: var(--ui-font-body);
  font-weight: 700;
  line-height: 1.4;
  text-align: left;
  cursor: pointer;
}

.search-prototype-result-list .result-title:disabled {
  color: var(--app-muted);
  cursor: default;
}

.search-prototype-result-list .result-author,
.search-prototype-result-list .search-result-intro {
  font-size: var(--ui-font-caption);
  line-height: 1.5;
  color: var(--app-muted);
}

.search-prototype-result-list .result-title,
.search-prototype-result-list .result-author,
.search-prototype-result-list .search-result-intro,
.search-prototype-result-list .source-label {
  overflow: hidden;
  margin: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-prototype-result-list .search-result-side {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 160px;
}

.search-prototype-result-list .source-label { max-width: 120px; }

.search-prototype-results .advanced-search-toolbar,
.search-prototype-results .search-batch-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 10px 0;
}

.advanced-search-toolbar .source-search {
  display: flex;
  flex: 1 1 140px;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 100%;
}
.advanced-search-toolbar .source-search input {
  flex: 1;
  min-width: 0;
  width: 100%;
}
.advanced-search-toolbar select { min-width: 0; max-width: 100%; }

.search-loading,
.empty-card {
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

@media (max-width: 860px) {
  .search-prototype-layout {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    margin: 0;
  }

  .search-type-tabs {
    gap: 4px;
    padding: 8px 12px;
  }

  .search-type-options button,
  .search-scope-toggle {
    padding-inline: 8px;
  }

  .search-scope-toggle {
    display: grid;
    color: var(--app-muted);
    font-size: var(--ui-font-caption);
  }

  .search-prototype-layout>.source-scope-card {
    position: static;
    display: none;
    max-height: 50dvh;
    padding: 12px;
  }

  .search-prototype-layout>.source-scope-card.mobile-open {
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
    max-width: 92px;
  }

  .search-prototype-result-list .source-label { max-width: 92px; }
}

@media (max-width: 530px) {
  .search-prototype-topbar {
    padding-inline: 12px;
  }
}
</style>
