<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import type { SearchBookResult, SearchType } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import SearchHistory from "./SearchHistory.vue";
import { history, removeSearchHistory } from "../settings/historyStore";
import { SEARCH_RESULT_ROW_HEIGHT, useSearchResultWindow } from "./useSearchResultWindow";
import {
  searchType as selectedContentType, searchSources, selectedSourceIds, searchKeyword,
  searchBusy, searchProgress, searchErrors, searchResults,
  searchWasRun, searchHasMore, searchComplete, loadMoreSearchResults,
  activeSearchCancellationPending, runSearch, cancelActiveSearch,
  precisionSearch,
} from "./searchState";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import { useVirtualList } from "../../ui/useVirtualList";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import router, { routeNames } from "../../router";

defineOptions({ name: "SearchPage" });

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


const searchHistoryEntries = computed(() => history.value.search.entries);

function removeSearchHistoryEntry(query?: string): void {
  void removeSearchHistory(query).catch(error => notify(errorText(error), "error"));
}

function startNewSearch(): void {
  if (shelfBatchRecoveryRequired.value) return;
  scrollResultsToStart();
  clearResultFilters();
  void runSearch();
}

function updateKeyword(event: Event): void {
  searchKeyword.value = (event.target as HTMLInputElement).value;
}

function submitSearch(): void {
  closeSearchOptions();
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

const openingResultDetails = ref(false);
const sourceFilter = ref("");
const searchOptionsOpen = ref(false);
const searchOptionsElement = ref<HTMLElement | null>(null);
const searchOptionsButton = ref<HTMLButtonElement | null>(null);
const searchSourceQuery = ref("");
const searchSourceViewport = ref<HTMLElement | null>(null);
const filteredSearchSources = computed(() => {
  const query = searchSourceQuery.value.trim().toLocaleLowerCase();
  return query ? searchSources.value.filter(source => source.name.toLocaleLowerCase().includes(query)) : searchSources.value;
});
const sourceWindow = useVirtualList(filteredSearchSources, searchSourceViewport, 44);
const { range: searchSourceRange, visibleItems: visibleSearchSources } = sourceWindow;
watch(searchSourceQuery, () => { void sourceWindow.reset(); });

function closeSearchOptions(restoreFocus = false): void {
  searchOptionsOpen.value = false;
  if (restoreFocus) searchOptionsButton.value?.focus({ preventScroll: true });
}

async function toggleSearchOptions(): Promise<void> {
  if (searchOptionsOpen.value) { closeSearchOptions(); return; }
  searchOptionsOpen.value = true;
  await nextTick();
  searchOptionsElement.value?.querySelector<HTMLInputElement>("input:not(:disabled)")?.focus({ preventScroll: true });
}

function onSearchOptionsFocusOut(event: FocusEvent): void {
  if (event.relatedTarget instanceof Node && !searchOptionsElement.value?.contains(event.relatedTarget)) {
    closeSearchOptions();
  }
}

useOutsidePointerDismiss(
  () => searchOptionsOpen.value,
  target => target instanceof Node && Boolean(searchOptionsElement.value?.contains(target)),
  closeSearchOptions,
);
watch(() => router.currentRoute.value.name, () => closeSearchOptions());
const searchErrorsOpen = ref(false);
const failedSourceCount = computed(() => new Set(searchErrors.value.flatMap(error => error.sourceId ? [error.sourceId] : [])).size);
watch(() => searchErrors.value.length, count => { if (!count) searchErrorsOpen.value = false; }, { flush: "sync" });
function onSearchErrorsToggle(event: Event): void {
  const element = event.target as HTMLDetailsElement;
  if (element.isConnected) searchErrorsOpen.value = element.open;
}
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
  return searchResults.value.filter((result) => {
    return (resultContentType(result) === selectedContentType.value)
      && (!sourceFilter.value || result.sourceId === sourceFilter.value
        || result.sources?.some((source) => source.sourceId === sourceFilter.value));
  });
});

const resultListElement = ref<HTMLElement | null>(null);
const { range: resultWindow, renderedResults,
  rememberPosition: rememberResultPosition, forgetPosition: forgetResultPosition, scrollToStart: scrollResultsToStart,
} = useSearchResultWindow(visibleSearchResults, resultListElement);
watch(sourceFilter, scrollResultsToStart, { flush: "post" });

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

function clearResultFilters(): void {
  sourceFilter.value = "";
}
watch(selectedContentType, clearResultFilters);
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
        <div ref="searchOptionsElement" class="search-more-control" @keydown.esc.stop.prevent="closeSearchOptions(true)"
          @focusout="onSearchOptionsFocusOut">
          <button ref="searchOptionsButton" class="search-more-button" :class="{ 'has-notice': searchErrors.length || sourceFilter }"
            type="button" :aria-label="searchErrors.length ? '搜索选项，有搜索异常' : '搜索选项'" title="搜索选项"
            :aria-expanded="searchOptionsOpen" aria-controls="search-options-panel" @click="toggleSearchOptions">
            <PrototypeIcon name="more" />
          </button>
          <div v-if="searchOptionsOpen" id="search-options-panel" class="search-options-panel" role="region" aria-label="搜索选项">
            <label class="search-precision">
              匹配程度
              <select v-model="precisionSearch"
                :disabled="activeSearchCancellationPending || shelfBatchRecoveryRequired"
                @change="searchWasRun && startNewSearch()">
                <option value="exact">100% 匹配关键词</option>
                <option value="precise">精准搜索</option>
                <option value="none">无</option>
              </select>
            </label>
            <section class="search-menu-section">
              <div class="search-sidebar-source-heading">
                <div class="search-sidebar-title-group">
                  <h3 class="search-prototype-sidebar-title">搜索来源</h3>
                  <span class="search-sidebar-count">{{ selectedSourceIds.length }}/{{ searchSources.length }}</span>
                </div>
                <button type="button" :disabled="shelfBatchRecoveryRequired || !searchSources.length"
                  @click="setAllSearchSources(!selectedAllSources)">{{ selectedAllSources ? '取消全选' : '全选' }}</button>
              </div>
              <input v-if="searchSources.length > 12" v-model="searchSourceQuery" class="search-source-query" type="search"
                aria-label="筛选搜索来源" placeholder="筛选来源名称" />
              <div v-if="searchSources.length" ref="searchSourceViewport" class="search-menu-sources">
                <div :ref="sourceWindow.list">
                <div aria-hidden="true" :style="{ height: `${searchSourceRange.top}px` }"></div>
                <label v-for="source in visibleSearchSources" :key="source.id" class="search-menu-source">
                  <input type="checkbox" :checked="selectedSourceSet.has(source.id)"
                    :disabled="shelfBatchRecoveryRequired" @change="toggleSearchSource(source.id)" />
                  <span :title="source.name">{{ source.name }}</span>
                </label>
                <div aria-hidden="true" :style="{ height: `${searchSourceRange.bottom}px` }"></div>
                <p v-if="!filteredSearchSources.length" class="search-sidebar-no-source">没有匹配的来源。</p>
                </div>
              </div>
              <div v-else class="search-sidebar-no-source">没有已启用的源。
                <button type="button" @click="openSources">管理源</button>
              </div>
            </section>
            <section v-if="searchResults.length && (resultSources.length > 1 || sourceFilter)" class="search-menu-section">
              <label class="search-menu-filter-label" for="search-result-source">结果来源</label>
              <div class="search-list-actions">
                <select id="search-result-source" v-model="sourceFilter" class="search-source-filter"
                  :disabled="shelfBatchRecoveryRequired">
                  <option value="">全部来源</option>
                  <option v-for="[sourceId, name] in resultSources" :key="sourceId" :value="sourceId">{{ name }}</option>
                </select>
                <button v-if="sourceFilter" class="search-list-utility" type="button" aria-label="清除来源筛选"
                  title="清除来源筛选" :disabled="shelfBatchRecoveryRequired" @click="clearResultFilters">
                  <PrototypeIcon name="close" />
                </button>
              </div>
            </section>
            <details v-if="searchErrors.length" class="search-errors" :open="searchErrorsOpen" @toggle="onSearchErrorsToggle">
              <summary>
                <span>{{ failedSourceCount ? `${failedSourceCount} 个源未完成搜索` : `搜索异常（${searchErrors.length} 条）` }}</span>
                <PrototypeIcon name="right" />
              </summary>
              <ul v-if="searchErrorsOpen">
                <li v-for="(error, index) in searchErrors" :key="`${error.sourceId}-${index}`">
                  <b>{{ error.sourceId ? sourceNameForId(error.sourceId) : '源' }}</b>：{{ error.message }}
                </li>
              </ul>
            </details>
          </div>
        </div>
      </div>
    </header>
    <div class="search-prototype-layout">
      <nav class="search-type-tabs" aria-label="搜索类型">
        <div class="search-type-options">
          <button v-for="type in contentTypes" :key="type.id" type="button"
            :class="{ active: selectedContentType === type.id }" :aria-pressed="selectedContentType === type.id"
            :disabled="shelfBatchRecoveryRequired" @click="selectedContentType = type.id">{{ type.label }}</button>
        </div>
      </nav>
      <div v-if="searchWasRun && (searchBusy || activeSearchCancellationPending || (searchProgress && !searchComplete))" class="search-status-strip">
        <span class="search-list-progress" role="status">{{ searchProgress }}</span>
        <button v-if="searchBusy || activeSearchCancellationPending" type="button" class="text-button"
          :disabled="activeSearchCancellationPending" @click="cancelActiveSearch">{{ activeSearchCancellationPending ? '取消中…' : '取消搜索' }}</button>
      </div>
      <main class="search-prototype-results" @wheel.passive="onSearchWheel" @touchstart.passive="onSearchTouchStart"
        @touchmove.passive="onSearchTouchMove" @keydown="onSearchKeyDown">
        <SearchHistory v-if="!searchWasRun && searchHistoryEntries.length" :entries="searchHistoryEntries"
          @use-query="reuseSearchQuery" @delete-query="removeSearchHistoryEntry"
          @clear-all="removeSearchHistoryEntry()" />
        <div v-else-if="!searchWasRun" class="page-empty-state search-start-state">
          <span class="empty-state-glyph"><PrototypeIcon name="search" /></span>
          <strong>{{ searchSources.length ? '找到下一本想读的书' : '先添加内容来源' }}</strong>
          <p>{{ searchSources.length ? '输入书名、作者或关键词，开始搜索。' : '添加并启用来源后，就可以搜索和阅读。' }}</p>
          <button v-if="!searchSources.length" type="button" class="button primary" @click="openSources">管理来源</button>
        </div>
        <div class="results-section">
          <template v-if="searchWasRun">
            <p v-if="searchBusy && !searchResults.length" class="search-empty-inline" role="status">正在搜索…</p>
            <template v-else-if="searchResults.length">
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
              <p v-if="!visibleSearchResults.length && !searchBusy" class="search-empty-inline">暂无匹配结果</p>
            </template>
            <p v-else-if="!searchBusy" class="search-empty-inline">暂无匹配结果</p>
            <div ref="loadMoreMarker" class="search-load-more" aria-hidden="true"></div>
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
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  width: min(1120px, 100%);
  min-width: 0;
}

.search-back-button {
  width: 36px;
  min-width: 36px;
  height: var(--ui-control-height);
  min-height: var(--ui-control-height);
  border-radius: 10px;
  color: var(--app-text);
}

.search-prototype-top-input {
  display: flex;
  align-items: center;
  flex: 1;
  gap: 4px;
  min-width: 0;
  height: var(--ui-control-height);
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
.clear-search:focus-visible,
.search-type-options button:focus-visible,
.search-more-button:focus-visible,
.search-list-utility:focus-visible,
.search-source-filter:focus-visible,
.search-sidebar-source-heading button:focus-visible {
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

.search-prototype-layout {
  display: flex;
  flex-direction: column;
  width: min(1120px, calc(100% - 32px));
  margin: 12px auto;
}

.search-more-control {
  flex: none;
}

.search-more-button {
  position: relative;
  display: grid;
  place-items: center;
  width: 36px;
  height: var(--ui-control-height);
  border-radius: var(--ui-radius);
  color: var(--app-muted);
  cursor: pointer;
}

.search-more-button[aria-expanded="true"] {
  color: var(--app-text);
  background: var(--app-panel-soft);
}

.search-more-button :deep(svg) {
  width: var(--ui-icon-size);
  height: var(--ui-icon-size);
}

.search-more-button.has-notice::after {
  content: "";
  position: absolute;
  top: 8px;
  right: 5px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--app-accent);
}

.search-options-panel {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  z-index: 4;
  width: 320px;
  max-width: 100%;
  max-height: min(560px, calc(100dvh - var(--safe-top) - 88px));
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 8px 12px;
  border: 1px solid var(--app-line);
  border-radius: 12px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
}

.search-menu-section {
  display: grid;
  gap: 8px;
  padding-block: 8px;
  border-top: 1px solid var(--app-line);
}

.search-menu-sources {
  max-height: 240px;
  overflow-y: auto;
  overscroll-behavior: contain;
}
.search-menu-source { height: 44px; }
.search-source-query { width: 100%; min-height: 40px; padding: 0 10px; border: 1px solid var(--app-line); border-radius: 8px; color: var(--app-text); background: var(--app-panel-softer); font: inherit; }
.search-status-strip { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 0 var(--ui-page-gutter); min-height: 44px; border-bottom: 1px solid var(--app-line); }
.search-start-state { min-height: 42dvh; }

.search-menu-filter-label {
  font-size: var(--ui-font-caption);
  font-weight: 600;
}

.search-prototype-sidebar-title {
  margin: 0;
  font-size: var(--ui-font-body);
  font-weight: 600;
}

.search-sidebar-source-heading button,
.search-sidebar-no-source button {
  border: 0;
  background: transparent;
  text-align: left;
  min-height: var(--ui-compact-control-height);
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  cursor: pointer;
}

.search-sidebar-title-group,
.search-sidebar-source-heading,
.search-list-actions,
.search-precision {
  display: flex;
  align-items: center;
  gap: 8px;
}

.search-sidebar-source-heading {
  justify-content: space-between;
}

.search-sidebar-count,
.search-list-progress {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-precision {
  min-height: var(--ui-control-height);
  gap: 6px;
  cursor: pointer;
  font-size: var(--ui-font-body);
  white-space: nowrap;
}

.search-precision input {
  width: 14px;
  height: 14px;
  margin: 0;
  accent-color: var(--app-accent);
}

.search-menu-source {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 36px;
  font-size: var(--ui-font-body);
  cursor: pointer;
}

.search-menu-source input {
  flex: none;
  width: 14px;
  height: 14px;
  margin: 0;
  opacity: 1;
  accent-color: var(--app-accent);
}

.search-menu-source span {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-load-more {
  padding: 14px 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  text-align: center;
}

.search-load-more:empty {
  padding: 0;
  min-height: 1px;
}

.search-sidebar-no-source {
  font-size: var(--ui-font-caption);
}

.search-type-tabs {
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

.search-type-options button {
  display: grid;
  place-items: center;
  min-height: 36px;
  flex: none;
  padding: 0 12px;
  border: 0;
  border-radius: var(--ui-radius);
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

.search-prototype-results {
  min-width: 0;
  max-width: 100%;
}

.search-list-actions {
  min-width: 0;
  gap: 4px;
}

.search-source-filter {
  flex: 1;
  width: 100%;
  max-width: 100%;
  min-width: 0;
  height: var(--ui-compact-control-height);
  padding: 0 8px;
  border: 1px solid var(--app-line);
  border-radius: var(--ui-radius);
  color: var(--app-text);
  background: var(--app-surface);
  font-size: var(--ui-font-caption);
}

.search-progress-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.search-list-progress {
  min-width: 0;
  margin: 8px 0;
}

.search-progress-row .search-list-progress {
  margin: 0;
}

.search-list-utility {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  gap: 6px;
  min-width: var(--ui-compact-control-height);
  min-height: var(--ui-compact-control-height);
  padding: 0 9px;
  border: 0;
  border-radius: 8px;
  color: var(--app-muted);
  background: transparent;
  font-size: var(--ui-font-caption);
  white-space: nowrap;
  cursor: pointer;
}

.search-list-utility :deep(svg) {
  width: 16px;
  height: 16px;
}

.search-empty-inline {
  margin: 0;
  padding: 24px 0;
  text-align: center;
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

.search-errors {
  border-top: 1px solid var(--app-line);
  min-width: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-errors summary { display: flex; align-items: center; gap: 8px; min-height: 36px; padding: 4px 0; list-style: none; cursor: pointer; }
.search-errors summary::-webkit-details-marker { display: none; }
.search-errors summary > span { flex: 1; min-width: 0; }
.search-errors summary :deep(svg) { width: 14px; height: 14px; }
.search-errors[open] summary :deep(svg) { transform: rotate(90deg); }
.search-errors summary:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
.search-errors ul {
  max-height: min(260px, 40dvh);
  overflow-y: auto;
  overscroll-behavior: contain;
  margin: 4px 0 0;
  padding: 10px 12px 10px 28px;
  border: 1px solid var(--app-line);
  border-radius: 8px;
  background: var(--app-panel-softer);
  line-height: 1.6;
  overflow-wrap: anywhere;
}
.search-errors li + li { margin-top: 6px; }

@media (max-width: 860px) {
  .search-prototype-layout {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    margin: 0;
  }

  .search-type-tabs {
    padding: 12px 16px 8px;
  }

  .search-type-options button {
    flex: 1;
    padding-inline: 10px;
  }

  .search-prototype-results {
    box-sizing: border-box;
    padding: 0 16px 24px;
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

</style>
