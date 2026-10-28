<script setup lang="ts">
import { Searchbar as KSearchbar, Preloader as KPreloader, Popover as KPopover, Checkbox as KCheckbox, Button as KButton } from "konsta/vue";
import { computed, onBeforeUnmount, ref, watch } from "vue";
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
import SubpageHeader from "../../ui/SubpageHeader.vue";
import SelectMenu from "../../ui/SelectMenu.vue";
import { useVirtualList } from "../../ui/useVirtualList";
import { popoverOwner, useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
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
const searchOptionsButton = ref<InstanceType<typeof KButton> | null>(null);
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
  if (restoreFocus) searchOptionsButton.value?.$el.focus({ preventScroll: true });
}

function toggleSearchOptions(): void {
  if (searchOptionsOpen.value) { closeSearchOptions(); return; }
  searchOptionsOpen.value = true;
}

function onSearchOptionsFocusOut(event: FocusEvent): void {
  const target = popoverOwner(event.relatedTarget) ?? event.relatedTarget;
  if (target instanceof Node && !searchOptionsElement.value?.contains(target) && !document.getElementById("search-options-panel")?.contains(target)) {
    closeSearchOptions();
  }
}

useOutsidePointerDismiss(
  () => searchOptionsOpen.value,
  target => target instanceof Node && Boolean(searchOptionsElement.value?.contains(target) || document.getElementById("search-options-panel")?.contains(target)),
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
    <SubpageHeader title="搜索" class="search-prototype-topbar" back-label="返回" @back="returnFromSearch">
<template #actions><div ref="searchOptionsElement" class="search-more-control" @keydown.esc.stop.prevent="closeSearchOptions(true)"
          @focusout="onSearchOptionsFocusOut">
          <KButton rounded inline clear ref="searchOptionsButton" class="search-more-button size-11 p-0"
            :class="{ 'has-notice': searchErrors.length || sourceFilter }" type="button"
            :aria-label="searchErrors.length ? '搜索选项，有搜索异常' : '搜索选项'" title="搜索选项" :aria-expanded="searchOptionsOpen"
            aria-controls="search-options-panel" @click="toggleSearchOptions">
            <PrototypeIcon name="tune" />
          </KButton>

        </div></template></SubpageHeader>
<KPopover v-if="searchOptionsOpen" opened :backdrop="false" target=".search-more-button" class="search-options-panel w-[min(360px,calc(100vw-32px))]">
  <div id="search-options-panel" role="region" aria-label="搜索选项" class="p-4 max-h-[calc(100dvh-100px)] overflow-auto">
<div class="search-options-heading">
              <strong>搜索设置</strong>
              <KButton rounded inline clear type="button" class="search-list-utility gap-2" aria-label="关闭搜索设置" @click="closeSearchOptions(true)">
                <PrototypeIcon name="close" />
              </KButton>
            </div><div class="search-precision">
              匹配程度
              <SelectMenu v-model="precisionSearch" label="匹配程度" :disabled="activeSearchCancellationPending || shelfBatchRecoveryRequired"
                :options="[{ value: 'exact', label: '完全匹配' }, { value: 'precise', label: '精准搜索' }, { value: 'none', label: '不限' }]"
                @update:model-value="searchWasRun && startNewSearch()" />
            </div><section class="search-menu-section">
              <div class="search-sidebar-source-heading">
                <div class="search-sidebar-title-group">
                  <h3 class="search-prototype-sidebar-title">搜索来源</h3>
                  <span class="search-sidebar-count">{{ selectedSourceIds.length }}/{{ searchSources.length }}</span>
                </div>
                <KButton rounded inline clear class="gap-2 text-current" type="button" :disabled="shelfBatchRecoveryRequired || !searchSources.length"
                  @click="setAllSearchSources(!selectedAllSources)">{{ selectedAllSources ? '取消全选' : '全选' }}</KButton>
              </div>
              <KSearchbar v-if="searchSources.length > 12" class="m-0! w-full! p-0!" aria-label="筛选搜索来源" placeholder="筛选来源名称"
                :disable-button="false" :value="searchSourceQuery" @input="searchSourceQuery = ($event.target as HTMLInputElement).value" @clear="searchSourceQuery = ''" />
              <div v-if="searchSources.length" ref="searchSourceViewport" class="search-menu-sources">
                <div :ref="sourceWindow.list">
                  <div aria-hidden="true" :style="{ height: `${searchSourceRange.top}px` }"></div>
                  <label v-for="source in visibleSearchSources" :key="source.id" class="search-menu-source">
                    <KCheckbox component="span" :checked="selectedSourceSet.has(source.id)"
                      :disabled="shelfBatchRecoveryRequired" @change="toggleSearchSource(source.id)"  />
                    <span :title="source.name">{{ source.name }}</span>
                  </label>
                  <div aria-hidden="true" :style="{ height: `${searchSourceRange.bottom}px` }"></div>
                  <p v-if="!filteredSearchSources.length" class="search-sidebar-no-source">没有匹配的来源。</p>
                </div>
              </div>
              <div v-else class="search-sidebar-no-source">没有已启用的来源。
                <KButton rounded inline clear class="gap-2 text-current" type="button" @click="openSources">管理源</KButton>
              </div>
            </section><section v-if="searchResults.length && (resultSources.length > 1 || sourceFilter)"
              class="search-menu-section">
              <label class="search-menu-filter-label" for="search-result-source">结果来源</label>
              <div class="search-list-actions">
                <SelectMenu id="search-result-source" v-model="sourceFilter" class="search-source-filter" label="结果来源"
                  :disabled="shelfBatchRecoveryRequired" :options="[{ value: '', label: '全部来源' }, ...resultSources.map(([value, label]) => ({ value, label }))]" />
                <KButton rounded inline clear v-if="sourceFilter" class="search-list-utility gap-2 text-current" type="button" aria-label="清除来源筛选" title="清除来源筛选"
                  :disabled="shelfBatchRecoveryRequired" @click="clearResultFilters">
                  <PrototypeIcon name="close" />
                </KButton>
              </div>
            </section><details v-if="searchErrors.length" class="search-errors" :open="searchErrorsOpen"
              @toggle="onSearchErrorsToggle">
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
</KPopover>
    <div class="search-prototype-layout">
      <form class="search-form flex items-center gap-3 px-[var(--ui-page-gutter)] py-3" @submit.prevent="submitSearch">
        <label class="sr-only" for="search-keyword">搜索关键词</label>
        <KSearchbar input-id="search-keyword" class="m-0! w-full! min-w-0 p-0!" :value="searchKeyword" placeholder="搜索标题、作者或关键词"
          :disable-button="false" :inert="shelfBatchRecoveryRequired" @input="updateKeyword" @clear="searchKeyword = ''" />
        <KButton rounded inline type="submit" class="shrink-0" :disabled="!searchKeyword.trim() || activeSearchCancellationPending || shelfBatchRecoveryRequired">搜索</KButton>
      </form>
      <nav class="search-type-tabs" aria-label="搜索类型">
        <div class="search-type-options">
          <KButton rounded :tonal="selectedContentType === type.id" :clear="!(selectedContentType === type.id)" inline class="gap-2" v-for="type in contentTypes" :key="type.id" type="button"
            :class="{ active: selectedContentType === type.id }" :aria-pressed="selectedContentType === type.id"
            :disabled="shelfBatchRecoveryRequired" @click="selectedContentType = type.id">{{ type.label }}</KButton>
        </div>
        <span class="search-source-summary" :title="`已选择 ${selectedSourceIds.length} 个搜索来源`">
          {{ selectedSourceIds.length }} 个源
        </span>
      </nav>
      <div v-if="searchWasRun" class="search-status-strip">
        <span class="search-list-progress" role="status">
          <span v-if="searchBusy || activeSearchCancellationPending" class="search-status-dot"
            aria-hidden="true"></span>
          {{ searchComplete ? `找到 ${visibleSearchResults.length} 本` : searchProgress }}
        </span>
        <KButton rounded inline clear v-if="searchBusy || activeSearchCancellationPending" type="button" class="search-cancel gap-2"
          :disabled="activeSearchCancellationPending" @click="cancelActiveSearch">{{ activeSearchCancellationPending ?
            '取消中…' : '取消搜索' }}</KButton>
        <KButton rounded inline clear v-if="sourceFilter" type="button" class="search-active-filter gap-2" :disabled="shelfBatchRecoveryRequired"
          :aria-label="`清除来源筛选：${sourceNameForId(sourceFilter)}`" @click="clearResultFilters">
          <span>{{ sourceNameForId(sourceFilter) }}</span>
          <PrototypeIcon name="close" />
        </KButton>
      </div>
      <main class="search-prototype-results" @wheel.passive="onSearchWheel" @touchstart.passive="onSearchTouchStart"
        @touchmove.passive="onSearchTouchMove" @keydown="onSearchKeyDown">
        <SearchHistory v-if="!searchWasRun && searchSources.length && searchHistoryEntries.length"
          :entries="searchHistoryEntries" :busy="shelfBatchRecoveryRequired" @use-query="reuseSearchQuery"
          @delete-query="removeSearchHistoryEntry" @clear-all="removeSearchHistoryEntry()" />
        <div v-else-if="!searchWasRun" class="page-empty-state">
          <span class="empty-state-glyph"><PrototypeIcon name="search" /></span>
          <strong>{{ searchSources.length ? '输入书名或作者开始搜索' : '请先添加内容来源' }}</strong>
          <KButton v-if="!searchSources.length" rounded inline @click="openSources">管理来源</KButton>
        </div>
        <div class="results-section">
          <template v-if="searchWasRun">
            <div v-if="searchBusy && !searchResults.length" class="page-empty-state" role="status"><KPreloader />正在搜索…</div>
            <template v-else-if="searchResults.length">
              <div v-if="visibleSearchResults.length" ref="resultListElement"
                class="search-results-grid search-prototype-result-list" role="list"
                :style="{ '--search-result-row-height': `${SEARCH_RESULT_ROW_HEIGHT}px` }">
                <div aria-hidden="true" :style="{ height: `${resultWindow.top}px` }"></div>
                <article v-for="(result, index) in renderedResults" :key="result.resultId" role="listitem"
                  :aria-posinset="resultWindow.start + index + 1" :aria-setsize="visibleSearchResults.length">
                  <button class="result-card" type="button"
                    :disabled="shelfBatchRecoveryRequired || openingResultDetails" :aria-label="`打开《${result.title}》`"
                    @click="openSearchResultDetails(result)">
                    <span class="result-cover">
                      <img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" decoding="async" alt="" />
                      <span v-else class="search-abstract-cover">{{ result.title.slice(0, 1) }}</span>
                    </span>
                    <span class="result-details">
                      <span class="result-heading">
                        <strong class="result-title">{{ result.title }}</strong>
                        <small class="source-label" :title="resultSourceLabel(result)">{{ resultSourceLabel(result)
                          }}</small>
                      </span>
                      <span class="result-meta">
                        <span class="result-author">{{ result.author || '作者未知' }}</span>
                        <small v-if="result.responseTimeMs != null" class="result-latency"
                          :title="`响应耗时 ${result.responseTimeMs} 毫秒`">{{ result.responseTimeMs }} ms</small>
                      </span>
                      <span v-if="result.intro" class="search-result-intro">{{ String(result.intro) }}</span>
                      <span v-else-if="result.latestChapter" class="search-result-intro">{{ result.latestChapter
                        }}</span>
                    </span>
                  </button>
                </article>
                <div aria-hidden="true" :style="{ height: `${resultWindow.bottom}px` }"></div>
              </div>
            </template>
            <div v-if="!searchBusy && !visibleSearchResults.length" class="page-empty-state">
              {{ sourceFilter ? '该来源暂无匹配内容' : '暂无匹配内容' }}
              <KButton rounded inline clear v-if="sourceFilter" type="button" class="gap-2" :disabled="shelfBatchRecoveryRequired"
                @click="clearResultFilters">清除筛选</KButton>
            </div>
            <div ref="loadMoreMarker" class="search-load-more" aria-hidden="true"></div>
          </template>
        </div>
      </main>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.search-page-prototype {
  --search-width: 920px;
  --search-gutter: 20px;
  width: 100%;
  min-width: 0;
  overflow-x: clip;
  padding-bottom: calc(max(24px, var(--safe-bottom)) + var(--ui-playback-inset));
}

.search-more-button {
  flex: none;
  width: 36px;
  min-width: 36px;
  height: 44px;
  min-height: 44px;
  border-radius: 10px;
  color: var(--app-muted);
  cursor: pointer;
}

.search-more-control {
  flex: none;
}

.search-more-button {
  position: relative;
}

.search-more-button :deep(svg) {
  width: 22px;
  height: 22px;
}

.search-more-button.has-notice::after {
  content: "";
  position: absolute;
  top: 7px;
  right: 4px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--app-accent);
}

.search-prototype-layout {
  max-width: var(--app-content-width);
  width: 100%;
  margin-inline: auto;
}

.search-type-tabs, .search-type-options {
  display: flex;
  align-items: center;
  min-width: 0;
}

.search-type-tabs {
  gap: 20px;
  padding-inline: var(--ui-page-gutter);
  border-bottom: 1px solid var(--app-line);
}

.search-type-options {
  flex: 1;
  gap: 20px;
}

.search-type-options button {
  position: relative;
  min-height: 48px;
  padding-inline: 6px;
  white-space: nowrap;
}

.search-type-options button.active {
  color: var(--app-accent-ink);
  font-weight: 700;
}

.search-source-summary {
  display: flex;
  align-items: center;
  flex: none;
  min-height: 44px;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  font-variant-numeric: tabular-nums;
}

.search-status-strip {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  column-gap: 12px;
  min-height: 48px;
  padding-block: 6px;
  padding-inline: var(--ui-space-3);
}

.search-list-progress {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  overflow-wrap: anywhere;
}

.search-status-dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--app-accent);
}

.search-cancel {
  flex: none;
  min-height: 36px;
}

.search-active-filter {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  min-height: 32px;
  padding: 4px 8px;
}

.search-active-filter span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-active-filter :deep(svg) {
  width: 14px;
  height: 14px;
}

.search-prototype-results {
  min-width: 0;
}

.search-prototype-results > .search-history {
  padding-inline: var(--ui-space-3);
}

.search-load-more {
  height: 1px;
}

.search-options-heading, .search-sidebar-source-heading, .search-sidebar-title-group, .search-list-actions, .search-precision {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.search-options-heading, .search-sidebar-source-heading, .search-precision {
  justify-content: space-between;
}

.search-options-heading {
  min-height: 40px;
}

.search-options-heading strong {
  font-size: var(--ui-font-body);
}

.search-list-utility {
  width: 32px;
  height: 32px;
}

.search-list-utility :deep(svg) {
  width: 18px;
  height: 18px;
}

.search-precision {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding-block: 12px;
  border-top: 1px solid var(--app-line);
  font-size: var(--ui-font-body);
}

.search-menu-section {
  display: grid;
  gap: 10px;
  padding-block: 12px;
  border-top: 1px solid var(--app-line);
}

.search-prototype-sidebar-title {
  margin: 0;
  font-size: var(--ui-font-body);
  font-weight: 600;
}

.search-sidebar-count {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-sidebar-source-heading button, .search-sidebar-no-source button {
  min-height: 32px;
}

.search-menu-sources {
  max-height: 220px;
  overflow-y: auto;
  overscroll-behavior: contain;
}

.search-menu-source {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 44px;
  padding-inline: 4px;
  font-size: var(--ui-font-body);
  cursor: pointer;
}

.search-menu-source span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-source-query {
  width: 100%;
}

.search-sidebar-no-source {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-menu-filter-label {
  font-size: var(--ui-font-body);
}

.search-source-filter {
  flex: 1;
  width: 100%;
}

.search-errors {
  min-width: 0;
  border-top: 1px solid var(--app-line);
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.search-errors summary {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 44px;
  list-style: none;
  cursor: pointer;
}

.search-errors summary::-webkit-details-marker {
  display: none;
}

.search-errors summary>span {
  flex: 1;
  min-width: 0;
}

.search-errors summary :deep(svg) {
  width: 14px;
  height: 14px;
}

.search-errors[open] summary :deep(svg) {
  transform: rotate(90deg);
}

.search-errors ul {
  max-height: min(240px, 35dvh);
  overflow-y: auto;
  overscroll-behavior: contain;
  margin: 0;
  padding: 8px 10px 8px 24px;
  border-radius: 8px;
  background: var(--app-panel-softer);
  line-height: 1.6;
  overflow-wrap: anywhere;
}

.search-errors li+li {
  margin-top: 8px;
}

.search-prototype-result-list {
  display: block;
  overflow-anchor: none;
}

.search-prototype-result-list .result-card {
  display: grid;
  grid-template-columns: 46px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  width: 100%;
  height: var(--search-result-row-height);
  min-height: var(--search-result-row-height);
  contain: layout paint;
  padding: 8px var(--ui-space-3);
  border-bottom: 1px solid var(--app-line);
  text-align: left;
  cursor: pointer;
}

.result-card:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: -2px;
}

.result-cover {
  width: 46px;
  height: 62px;
  overflow: hidden;
  border-radius: 4px;
}

.result-cover img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.search-abstract-cover {
  display: grid;
  place-items: center;
  width: 100%;
  height: 100%;
  background: var(--app-accent-soft);
  color: var(--app-accent-ink);
  font-size: var(--ui-font-heading);
}

.result-details {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 3px;
  min-width: 0;
}

.result-heading, .result-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-width: 0;
}

.result-title {
  flex: 1;
  min-width: 0;
  color: var(--app-text);
  font-size: var(--ui-font-body);
  font-weight: 700;
  line-height: 1.35;
}

.result-author, .search-result-intro, .source-label, .result-latency {
  font-size: var(--ui-font-caption);
  line-height: 1.4;
  color: var(--app-muted);
}

.result-title, .result-author, .search-result-intro, .source-label {
  overflow: hidden;
  margin: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.source-label {
  flex: none;
  max-width: 100px;
  color: var(--app-accent-ink);
}

.result-author {
  flex: 1;
  min-width: 0;
}

.result-latency {
  flex: none;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

@media (max-width: 600px) {
  .search-page-prototype {
    --search-gutter: 16px;
  }

  .search-type-tabs {
    gap: 12px;
  }

  .search-type-options {
    justify-content: space-between;
    gap: 0;
  }

  .search-type-options button {
    padding-inline: 10px;
  }

  .search-prototype-result-list .result-card {
    gap: 10px;
  }

  .source-label {
    max-width: 84px;
  }
}
}
</style>
