<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { addBook } from "../../api/books";
import { getHomeConfig, listDiscoveryBooks, listDiscoveryCategories } from "../../api/discovery";
import { readResource } from "../../api/resources";
import { appBootstrap } from "../../api/core";
import { shelfBatchRecoveryMessage, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import type {
  AppBootstrap,
  BookResource,
  DiscoveryCategoriesResource,
  HomeConfigDocument,
  ResourceDescriptor,
  SearchBookResult,
  SearchResource,
  ShelfResource,
  SourceDefinitionsResource,
  SourceMetadata,
} from "../../api/types";
import { useSearchResultActions } from "../discovery/useSearchResultActions";
import { discardBookDetailOpenRequest, requestBookDetailOpen, type BookDetailSearchResultActions } from "../books/bookDetailOpenRequest";
import router, { routeNames, settingsRouteNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import PrimaryNavigation from "../../ui/PrimaryNavigation.vue";

function openSearch(): void {
  void router.push({ name: routeNames.search });
}

function manageHome(): void {
  void router.push({ name: settingsRouteNames["home-config"] });
}

const homeConfig = ref<HomeConfigDocument>({
  schemaVersion: 1,
  tabs: [{ id: "tab-home", title: "\u4E3B\u9875", sortOrder: 0, sections: [] }],
});
const sectionResources = ref<Record<string, SearchResource>>({});
const loadingSectionIds = ref<string[]>([]);
const recommendations = ref<SearchBookResult[]>([]);
const sources = ref<SourceMetadata[]>([]);
const recommendationLoading = ref(false);
const recommendationError = ref("");
const configLoading = ref(true);
const loadError = ref("");
const restoreRevision = ref(0);
let pageRevision = 0;
let configRequestRevision = 0;
let sourceRequestRevision = 0;
let recommendationRevision = 0;
let unlisteners: UnlistenFn[] = [];
const sectionRequests = new Map<string, { sourceId: string; categoryId: string; token: symbol }>();

const enabledSources = computed(() => sources.value.filter((source) => source.enabled));
const busy = computed(() => configLoading.value);
const activeTabId = ref("");
const activeType = ref<"all" | "novel" | "comic" | "video" | "audio">("all");
const showSectionLayout = ref(false);
const typeFilters = [
  { id: "all", title: "\u5168\u90E8" },
  { id: "novel", title: "\u5C0F\u8BF4" },
  { id: "comic", title: "\u6F2B\u753B" },
  { id: "video", title: "\u89C6\u9891" },
  { id: "audio", title: "\u97F3\u9891" },
] as const;
const orderedTabs = computed(() => [...homeConfig.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
const gridResults = computed(() => {
  const seen = new Set<string>();
  const items = orderedSections.value.length
    ? orderedSections.value.flatMap((section) => sectionResources.value[section.id]?.results ?? [])
    : recommendations.value;
  return items.filter((item) => {
    const key = item.sourceId + ":" + (item.bookUrl ?? item.resultId);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).slice(0, 60);
});

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const detailSearchResults = ref<SearchBookResult[]>([]);
let shelfReadRevision = 0;
async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++shelfReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  const nextShelf = await readResource<ShelfResource>(descriptor);
  if (currentRestoreRevision !== restoreRevision.value || requestRevision !== shelfReadRevision) return;
  shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
}

const homeSearchResultActions = useSearchResultActions({
  searchResults: detailSearchResults,
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
const homeDetailActions: BookDetailSearchResultActions = {
  searchResultBatchBusy: homeSearchResultActions.searchResultBatchBusy,
  searchResultIsBusy: homeSearchResultActions.searchResultIsBusy,
  addSearchResult: homeSearchResultActions.addSearchResult,
};

async function openSearchResult(result: SearchBookResult): Promise<void> {
  if (homeSearchResultActions.searchResultBatchBusy.value || shelfBatchRecoveryRequired.value) return;
  detailSearchResults.value = [result];
  const requestId = requestBookDetailOpen({ kind: "search-result", result, actions: homeDetailActions });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

function contentType(result: SearchBookResult): "novel" | "comic" | "video" | "audio" {
  const declared = [result.contentType, result.mediaType, result.type]
    .find((value) => typeof value === "string");
  if (typeof declared === "string") {
    if (/^(comic|manga|\u6F2B\u753B)$/i.test(declared)) return "comic";
    if (/^(video|\u89C6\u9891)$/i.test(declared)) return "video";
    if (/^(audio|\u97F3\u9891)$/i.test(declared)) return "audio";
    if (/^(novel|text|\u5C0F\u8BF4)$/i.test(declared)) return "novel";
  }
  const source = sources.value.find((item) => item.id === result.sourceId);
  if (source?.mediaType === "video" || source?.mediaType === "audio") return source.mediaType;
  if (/\u6F2B\u753B|comic|manga/i.test(source?.group ?? "")) return "comic";
  return "novel";
}

const visibleResults = computed(() => gridResults.value.filter(
  (item) => activeType.value === "all" || contentType(item) === activeType.value,
));
const gridLoading = computed(() => busy.value || (orderedSections.value.length
  ? orderedSections.value.some((section) => loadingSectionIds.value.includes(section.id))
  : recommendationLoading.value));

async function refreshRecommendations(): Promise<void> {
  const revision = ++recommendationRevision;
  const restoredAtStart = restoreRevision.value;
  const availableSources = enabledSources.value
    .filter((source) => source.isRss !== true)
    .slice(0, 8);
  const isCurrent = () => revision === recommendationRevision && restoredAtStart === restoreRevision.value;
  recommendations.value = [];
  recommendationError.value = "";
  recommendationLoading.value = availableSources.length > 0;
  if (!availableSources.length) return;

  const books: SearchBookResult[] = [];
  const seen = new Set<string>();
  const failures: string[] = [];
  let successfulSources = 0;
  for (const source of availableSources) {
    if (!isCurrent()) return;
    try {
      const categoriesResponse = await listDiscoveryCategories(source.id);
      const categories = await readResource<DiscoveryCategoriesResource>(categoriesResponse.resource);
      if (!isCurrent()) return;
      const category = categories.categories.find((item) => Boolean(item.categoryId));
      if (!category?.categoryId) continue;
      const response = await listDiscoveryBooks(source.id, category.categoryId, 1);
      const document = await readResource<SearchResource>(response.resource);
      if (!isCurrent()) return;
      for (const result of document.results.slice(0, 15)) {
        const key = result.sourceId + ":" + (result.bookUrl ?? result.resultId);
        if (seen.has(key)) continue;
        seen.add(key);
        books.push(result);
      }
      if (document.results.length) successfulSources += 1;
      if (document.errors.length) {
        failures.push(source.name + "\uFF1A" + document.errors.map((item) => item.message).join("\uFF1B"));
      }
      recommendations.value = books.slice(0, 40);
    } catch (error) {
      if (!isCurrent()) return;
      failures.push(source.name + "\uFF1A" + errorText(error));
    }
    if (books.length >= 40 || successfulSources >= 4) break;
  }
  if (!isCurrent()) return;
  recommendationLoading.value = false;
  recommendationError.value = failures.join("\uFF1B");
}

async function loadSection(sectionId: string, sourceId: string, categoryId: string): Promise<void> {
  const belongsToCurrentConfig = () => homeConfig.value.tabs.some((tab) => tab.sections.some(
    (section) => section.id === sectionId && section.sourceId === sourceId && section.categoryId === categoryId,
  ));
  if (!belongsToCurrentConfig()) return;
  const ongoing = sectionRequests.get(sectionId);
  if (ongoing?.sourceId === sourceId && ongoing.categoryId === categoryId) return;
  const restoredAtStart = restoreRevision.value;
  const token = Symbol(sectionId);
  sectionRequests.set(sectionId, { sourceId, categoryId, token });
  const isCurrent = () => restoredAtStart === restoreRevision.value
    && sectionRequests.get(sectionId)?.token === token
    && belongsToCurrentConfig();
  loadingSectionIds.value = [...new Set([...loadingSectionIds.value, sectionId])];
  try {
    const response = await listDiscoveryBooks(sourceId, categoryId, 1);
    const resource = await readResource<SearchResource>(response.resource);
    if (!isCurrent()) return;
    sectionResources.value = { ...sectionResources.value, [sectionId]: resource };
  } catch (error) {
    if (!isCurrent()) return;
    sectionResources.value = {
      ...sectionResources.value,
      [sectionId]: {
        schemaVersion: 1,
        keyword: "",
        page: 1,
        results: [],
        errors: [{ message: errorText(error) }],
      },
    };
  } finally {
    if (sectionRequests.get(sectionId)?.token === token) {
      sectionRequests.delete(sectionId);
      loadingSectionIds.value = loadingSectionIds.value.filter((id) => id !== sectionId);
    }
  }
}

async function refreshHomeConfig(descriptor?: ResourceDescriptor): Promise<void> {
  const restoredAtStart = restoreRevision.value;
  const requestRevision = ++configRequestRevision;
  const isCurrent = () => restoredAtStart === restoreRevision.value && requestRevision === configRequestRevision;
  const resource = descriptor ?? await getHomeConfig();
  const document = await readResource<HomeConfigDocument>(resource);
  if (!isCurrent()) return;
  const tabs = Array.isArray(document.tabs)
    ? document.tabs.map((tab) => ({ ...tab, sections: Array.isArray(tab.sections) ? tab.sections : [] }))
    : [];
  const previousSections = new Map(homeConfig.value.tabs.flatMap((tab) => tab.sections.map((section) => [section.id, section] as const)));
  const currentSections = new Map(tabs.flatMap((tab) => tab.sections.map((section) => [section.id, section] as const)));
  const stillSameSection = (id: string) => {
    const previous = previousSections.get(id);
    const current = currentSections.get(id);
    return Boolean(previous && current && previous.sourceId === current.sourceId && previous.categoryId === current.categoryId);
  };
  homeConfig.value = { schemaVersion: document.schemaVersion ?? 1, tabs };
  sectionResources.value = Object.fromEntries(Object.entries(sectionResources.value).filter(([id]) => stillSameSection(id)));
  for (const [id, request] of sectionRequests) {
    const section = currentSections.get(id);
    if (!section || section.sourceId !== request.sourceId || section.categoryId !== request.categoryId) sectionRequests.delete(id);
  }
  loadingSectionIds.value = loadingSectionIds.value.filter((id) => sectionRequests.has(id));

  if (tabs[0]?.sections.length) {
    recommendationRevision += 1;
    recommendations.value = [];
    recommendationLoading.value = false;
    recommendationError.value = "";
    void (async () => {
      for (const section of tabs[0].sections.slice(0, 4)) {
        if (!isCurrent()) return;
        await loadSection(section.id, section.sourceId, section.categoryId);
      }
    })();
  } else {
    void refreshRecommendations();
  }
}

function resetForRestore(): void {
  pageRevision += 1;
  configRequestRevision += 1;
  sourceRequestRevision += 1;
  recommendationRevision += 1;
  recommendations.value = [];
  recommendationLoading.value = false;
  recommendationError.value = "";
  homeConfig.value = { schemaVersion: 1, tabs: [] };
  sectionRequests.clear();
  sectionResources.value = {};
  loadingSectionIds.value = [];
}

async function handleAppStateUpdated(bootstrap: AppBootstrap): Promise<void> {
  resetForRestore();
  restoreRevision.value += 1;
  homeSearchResultActions.resetSearchResultState();
  detailSearchResults.value = [];
  await Promise.all([refreshSources(bootstrap.sources), refreshConfig(), refreshShelf(bootstrap.shelf)]);
}
function selectTab(id: string): void {
  activeTabId.value = id;
  const tab = orderedTabs.value.find((item) => item.id === id);
  if (!tab?.sections.length && !(recommendations.value.length || recommendationLoading.value)) {
    void refreshRecommendations();
  }
  for (const section of tab?.sections.slice(0, 4) ?? []) {
    if (!Object.prototype.hasOwnProperty.call(sectionResources.value, section.id)) {
      void loadSection(section.id, section.sourceId, section.categoryId);
    }
  }
}

function refreshGrid(): void {
  if (!orderedSections.value.length) {
    void refreshRecommendations();
  } else {
    for (const section of orderedSections.value.slice(0, 4)) {
      void loadSection(section.id, section.sourceId, section.categoryId);
    }
  }
}

watch(sources, () => {
  if (!configLoading.value && !orderedSections.value.length) void refreshRecommendations();
}, { deep: true });
watch(orderedTabs, (tabs) => {
  if (!tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = tabs[0]?.id ?? "";
}, { immediate: true });

function sectionResults(sectionId: string): SearchBookResult[] {
  return sectionResources.value[sectionId]?.results ?? [];
}
function sectionIsLoading(sectionId: string): boolean {
  return loadingSectionIds.value.includes(sectionId);
}
function sectionHasPartialError(sectionId: string): boolean {
  return (sectionResources.value[sectionId]?.errors.length ?? 0) > 0;
}
function sectionHasLoaded(sectionId: string): boolean {
  return Object.prototype.hasOwnProperty.call(sectionResources.value, sectionId);
}

async function refreshConfig(descriptor?: ResourceDescriptor): Promise<void> {
  const revision = pageRevision;
  configLoading.value = true;
  loadError.value = "";
  try {
    await refreshHomeConfig(descriptor);
  } catch (error) {
    if (revision === pageRevision) loadError.value = errorText(error);
  } finally {
    if (revision === pageRevision) configLoading.value = false;
  }
}

async function refreshSources(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++sourceRequestRevision;
  const restoredAtStart = restoreRevision.value;
  const document = await readResource<SourceDefinitionsResource>(descriptor);
  if (requestRevision !== sourceRequestRevision || restoredAtStart !== restoreRevision.value) return;
  sources.value = (document.sources ?? []).map((entry) => {
    const source = { ...entry };
    Reflect.deleteProperty(source, "definitionJson");
    return source;
  });
}

onMounted(async () => {
  try {
    unlisteners.push(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", async (event) => {
      if (event.payload.kind === "homeConfig") await refreshConfig(event.payload.resource);
    }));
    unlisteners.push(await listen<ResourceDescriptor>("sources-updated", (event) => {
      void refreshSources(event.payload).catch((error) => { loadError.value = errorText(error); });
    }));
    unlisteners.push(await listen<ResourceDescriptor>("shelf-updated", (event) => {
      void refreshShelf(event.payload).catch((error) => notify(`书架刷新失败：${errorText(error)}`, "error"));
    }));
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", (event) => {
      void handleAppStateUpdated(event.payload).catch((error) => { loadError.value = errorText(error); });
    }));
  } catch (error) {
    loadError.value = errorText(error);
  }
  try {
    const bootstrap = await appBootstrap();
    await Promise.all([refreshSources(bootstrap.sources), refreshConfig(), refreshShelf(bootstrap.shelf)]);
  } catch (error) {
    loadError.value = errorText(error);
    configLoading.value = false;
  }
});

onBeforeUnmount(() => {
  pageRevision += 1;
  restoreRevision.value += 1;
  resetForRestore();
  for (const unlisten of unlisteners) unlisten();
  unlisteners = [];
});
</script>

<template>
  <div class="home-page-layout">
    <PrimaryNavigation />
    <main class="home-page-main">
      <section class="home-page">
    <div class="discover-command">
      <button class="discover-search-launch" type="button" @click="openSearch">
        <span class="discover-search-icon"><PrototypeIcon name="search"/></span>
        <span>搜索书名、作者、关键词或来源</span>
      </button>
    </div>
    <section class="home-discovery">
      <div class="discover-filter-bar">
        <div class="discover-type-filters">
          <button v-for="filter in typeFilters" :key="filter.id" type="button"
            :class="{ active: activeType === filter.id }"
            @click="activeType = filter.id">{{ filter.title }}</button>
        </div>
        <div class="discover-grid-actions">
          <button type="button" class="discover-utility-button" :disabled="busy || gridLoading"
title="刷新发现内容" @click="refreshGrid"><PrototypeIcon name="refresh"/></button>
          <button v-if="orderedSections.length" type="button" class="discover-utility-button"
@click="showSectionLayout = !showSectionLayout">{{ showSectionLayout ? '封面流' : '栏目' }}</button>
          <button type="button" class="discover-utility-button"
            :disabled="busy" @click="manageHome">管理</button>
        </div>
      </div>
      <div v-if="orderedTabs.length > 1" class="home-tabs">
        <button v-for="tab in orderedTabs" :key="tab.id"

          :class="{ active: activeTab?.id === tab.id }" @click="selectTab(tab.id)">{{ tab.title }}</button>
      </div>
      <template v-if="!showSectionLayout">
      <div v-if="visibleResults.length" class="discovery-content-grid">
        <article v-for="(result, index) in visibleResults" :key="`${result.sourceId}:${result.resultId}`"
          class="discovery-cover-card">
          <button type="button" class="discovery-cover-action"
            @click="openSearchResult(result)">
            <span class="discovery-cover-art" :class="`cover-tone-${index % 5}`">
              <img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" decoding="async" />
              <span v-else class="discovery-cover-placeholder"></span>
            </span>
            <span class="discovery-cover-copy">
              <strong>{{ result.title }}</strong>
              <small>{{ result.author || '作者未知' }}</small>
              <span>{{ typeFilters.find((item) => item.id === contentType(result))?.title }} · {{ result.sourceName }}</span>
            </span>
          </button>
        </article>
      </div>
      <div v-else-if="gridLoading" class="discover-grid-state">
        <span class="mini-loader"></span><span>正在从书源加载发现内容…</span>
      </div>
      <div v-else class="discover-grid-state discover-grid-empty">
        <span class="discover-state-symbol"><PrototypeIcon name="compass"/></span>
        <strong>{{ activeType !== 'all' && gridResults.length ? '当前类型还没有内容' : '暂无可显示的发现内容' }}</strong>
        <p>{{ activeType !== 'all' && gridResults.length ? '选择“全部”可以查看其他类型。' : !enabledSources.length ? '还没有启用的书源，可以先导入或启用书源。' : '可以刷新书源，或管理栏目选择其他分类。' }}</p>
        <button type="button" class="home-button home-button-secondary" @click="manageHome">管理栏目</button>
      </div>
      <p v-if="(recommendationError || loadError) && !orderedSections.length" class="discover-grid-warning">{{ loadError || recommendationError }}</p>
      </template>
      <p v-if="loadError && orderedSections.length" class="discover-grid-warning">{{ loadError }}</p>
      <div v-if="showSectionLayout && orderedSections.length" class="home-section-list">
        <article v-for="section in orderedSections" :key="section.id" class="home-section-card" :class="`home-section-style-${section.style}`">
          <header class="home-section-heading">
            <div><p class="home-eyebrow">{{ section.sourceName }} · {{ section.categoryName }}</p><h4>{{ section.title }}</h4></div>
            <button class="home-text-button" :disabled="busy || sectionIsLoading(section.id)" @click="loadSection(section.id, section.sourceId, section.categoryId)">{{ sectionIsLoading(section.id) ? '加载中…' : '更多' }} <span>→</span></button>
          </header>
          <div v-if="sectionIsLoading(section.id) && !sectionResults(section.id).length" class="section-resource-state"><span class="mini-loader"></span>正在加载分类内容…</div>
          <div v-else-if="!sectionResults(section.id).length" class="section-resource-state">
            <span>{{ sectionHasPartialError(section.id) ? '部分内容暂时不可用。' : sectionHasLoaded(section.id) ? '这个分类暂无内容。' : '这个分类还没有加载内容。' }}</span>
            <button class="home-text-button" :disabled="busy || sectionIsLoading(section.id)" @click="loadSection(section.id, section.sourceId, section.categoryId)">加载分类 <span>→</span></button>
          </div>
          <template v-else>
            <p v-if="sectionHasPartialError(section.id)" class="section-partial-error">部分来源未能加载，以下为当前可用内容。</p>
            <ol v-if="section.style === 1" class="section-rank-list">
              <li v-for="(result, index) in sectionResults(section.id).slice(0, 5)" :key="result.resultId">
                <span class="rank-number">{{ String(index + 1).padStart(2, "0") }}</span>
                <button class="rank-result" @click="openSearchResult(result)"><img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" /><span v-else class="section-cover-fallback">{{ result.title.slice(0, 1) }}</span><span class="result-copy"><strong>{{ result.title }}</strong><small>{{ result.author || result.latestChapter || '打开书籍详情' }}</small></span></button>
              </li>
            </ol>
            <div v-else class="section-cover-row" :class="{ 'section-cover-video': section.coverVideo, 'section-four-row': section.style === 3, 'section-infinite-grid': section.style === 2 }">
              <button v-for="result in sectionResults(section.id).slice(0, section.style === 2 ? 8 : 6)" :key="result.resultId" class="section-book-card" @click="openSearchResult(result)">
                <img v-if="result.coverSrc" :src="result.coverSrc" loading="lazy" /><span v-else class="section-cover-fallback">{{ result.title.slice(0, 1) }}</span>
                <span class="section-book-copy"><strong>{{ result.title }}</strong><small>{{ result.author || result.latestChapter || '打开书籍详情' }}</small></span>
              </button>
            </div>
          </template>
        </article>
      </div>
    </section>
      </section>
    </main>
  </div>
</template>

<style scoped>
/* 发现主视图按原型采用直接的封面流，个性化栏目保留为按需视图。 */
.home-page-layout { display:flex; width:100%; height:100dvh; min-height:0; overflow:hidden; padding-top:var(--safe-top); background:var(--app-background); }
.home-page-main { flex:1; min-width:0; min-height:0; overflow-x:hidden; overflow-y:auto; padding:0 clamp(20px,3.6vw,56px) var(--safe-bottom); }
.home-page { display:flex; max-width:1180px; flex-direction:column; gap:22px; margin:0 auto; padding:10px 0 26px; }
.discover-command { display:flex; justify-content:center; align-items:center; gap:8px; }
.discover-search-launch { display:flex; align-items:center; gap:8px; width:min(680px,100%); height:44px; padding:0 9px 0 12px; border:1px solid var(--app-line); border-radius:12px; background:var(--app-surface); color:var(--app-muted); cursor:pointer; text-align:left; }
.discover-search-launch:hover { border-color:#d4d6e3; box-shadow:0 2px 8px rgb(30 33 45 / 5%); }
.discover-search-launch>span:nth-child(2) { flex:1; overflow:hidden; font-size:14px; white-space:nowrap; text-overflow:ellipsis; }
.discover-search-icon { display:grid; flex:none; place-items:center; }
.discover-search-icon { color:var(--app-text); }
.discover-search-icon svg { width:18px; height:18px; }
.home-discovery { min-width:0; }
.discover-filter-bar { position:relative; display:flex; align-items:center; justify-content:center; min-height:36px; margin-bottom:18px; }
.discover-type-filters { display:flex; justify-content:center; align-items:center; flex-wrap:wrap; gap:4px; }
.discover-type-filters button { min-height:31px; padding:0 12px; border:0; border-radius:999px; background:transparent; color:#767b86; cursor:pointer; font-size:12px; }
.discover-type-filters button.active { background:#202126; color:white; font-weight:700; }
.discover-grid-actions { position:absolute; right:0; top:0; display:flex; gap:5px; align-items:center; }
.discover-utility-button { min-height:31px; padding:0 9px; border:1px solid transparent; border-radius:9px; background:transparent; color:#969ba7; font-size:11px; cursor:pointer; }
.discover-utility-button:hover { background:#eceef3; color:var(--app-accent); }
.discover-utility-button:disabled { opacity:.5; cursor:default; }
.discover-utility-button svg { width:16px; height:16px; }
.discovery-content-grid { display:grid; grid-template-columns:repeat(5,minmax(0,1fr)); gap:29px 24px; }
.discovery-cover-card { min-width:0; }
.discovery-cover-action { display:block; width:100%; padding:0; border:0; background:transparent; text-align:left; cursor:pointer; }
.discovery-cover-art { position:relative; display:block; width:100%; overflow:hidden; aspect-ratio:4/5; border-radius:13px; background:linear-gradient(140deg,#466f6d,#1b3235); transition:transform .15s,box-shadow .15s; }
.discovery-cover-action:hover .discovery-cover-art { transform:translateY(-2px); box-shadow:0 12px 30px #25263919; }
.discovery-cover-art img { width:100%; height:100%; object-fit:cover; display:block; }
.discovery-cover-placeholder { position:absolute; inset:0; overflow:hidden; }
.discovery-cover-placeholder::before { content:""; position:absolute; width:76%; height:72%; right:-28%; top:15%; border-radius:50%; background:#fff2; filter:blur(1px); }
.discovery-cover-placeholder::after { content:""; position:absolute; left:-25%; right:-20%; bottom:-37%; height:65%; border-radius:50%; background:#0003; }
.cover-tone-1 { background:linear-gradient(145deg,#84697e,#3d3248); }
.cover-tone-2 { background:linear-gradient(145deg,#77999b,#31534f); }
.cover-tone-3 { background:linear-gradient(145deg,#374b6a,#151e2d); }
.cover-tone-4 { background:linear-gradient(145deg,#8b657d,#342232); }
.discovery-cover-copy { display:flex; flex-direction:column; min-width:0; padding:8px 1px 0; gap:3px; }
.discovery-cover-copy strong { overflow:hidden; color:#24252d; font-size:13px; white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy small { overflow:hidden; color:#838896; font-size:11px; white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy>span { overflow:hidden; color:#959aa5; font-size:10px; white-space:nowrap; text-overflow:ellipsis; }
.discover-grid-state { min-height:230px; display:flex; align-items:center; justify-content:center; gap:12px; padding:24px; border:1px dashed #e2e4eb; border-radius:15px; color:#828896; background:#ffffff7a; }
.discover-grid-empty { flex-direction:column; text-align:center; gap:8px; }
.discover-grid-empty strong { font-size:15px; color:#40444f; }
.discover-grid-empty p { margin:0; font-size:12px; line-height:1.6; }
.discover-state-symbol { display:grid; place-items:center; width:42px; height:42px; border-radius:50%; background:var(--app-accent-soft); color:var(--app-accent); }
.discover-grid-empty > .home-button { margin-top:6px; }
.discover-grid-warning { font-size:11px; color:#af6555; overflow-wrap:anywhere; }
.home-tabs { display:flex; justify-content:center; gap:5px; margin:0 0 20px; overflow-x:auto; }
.home-tabs button { padding:8px 14px; border:0; border-radius:999px; background:transparent; color:#787e89; cursor:pointer; }
.home-tabs button.active { background:var(--app-accent-soft); color:var(--app-accent); font-weight:700; }
.home-eyebrow { margin:0 0 4px; color:#999da7; font-size:11px; }
.home-section-list { display:grid; gap:18px; margin-top:23px; }
.home-section-card { padding:14px; border:1px solid #e4e6ec; border-radius:14px; background:white; }
.home-section-heading { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:13px; }
.home-section-heading h4 { margin:0; font-size:15px; }
.home-text-button { padding:7px; border:0; background:transparent; color:var(--app-accent); cursor:pointer; }
.section-resource-state { display:flex; gap:8px; align-items:center; justify-content:center; min-height:70px; color:#828896; font-size:12px; }
.mini-loader { display:inline-block; width:16px; height:16px; border:2px solid #ddd8fb; border-top-color:var(--app-accent); border-radius:50%; animation:home-spin .8s linear infinite; }
@keyframes home-spin { to {transform:rotate(360deg)} }
.section-cover-row { display:flex; gap:12px; overflow-x:auto; }
.section-book-card { display:flex; flex-direction:column; flex:0 0 120px; min-width:0; border:0; padding:0; background:transparent; text-align:left; cursor:pointer; }
.section-book-card img,.section-book-card .section-cover-fallback { width:100%; aspect-ratio:4/5; object-fit:cover; border-radius:9px; }
.section-cover-fallback { display:grid; place-items:center; background:#e6e1fa; color:var(--app-accent); font-weight:700; }
.section-book-copy { display:grid; gap:4px; padding-top:7px; }
.section-book-copy strong,.section-book-copy small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.section-book-copy strong { font-size:12px; }
.section-book-copy small { font-size:10px; color:#838896; }
.section-four-row,.section-infinite-grid { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); }
.section-four-row .section-book-card,.section-infinite-grid .section-book-card { width:100%; }
.section-rank-list { padding:0; list-style:none; }
.section-rank-list li { display:flex; gap:14px; align-items:center; border-bottom:1px solid #eeeef2; padding:10px 0; }
.rank-result { display:flex; align-items:center; gap:10px; border:0; background:transparent; text-align:left; cursor:pointer; }
.rank-result img,.rank-result .section-cover-fallback { width:42px; height:56px; border-radius:6px; object-fit:cover; }
.rank-number { color:#969aa6; }
.result-copy { display:grid; gap:5px; }
.result-copy small { color:#858b94; }
@media(max-width:1120px) { .discovery-content-grid { grid-template-columns:repeat(4,minmax(0,1fr)); gap:24px 18px; } }
@media(max-width:860px) {
  .home-page-main { padding:0 14px calc(67px + var(--safe-bottom)); }
  .discovery-content-grid { grid-template-columns:repeat(3,minmax(0,1fr)); }
}
@media(max-width:640px) {
  .home-page { padding-top:0; }
  .discovery-content-grid { grid-template-columns:repeat(2,minmax(0,1fr)); gap:22px 11px; }
  .discover-command { justify-content:stretch; }
  .discover-search-launch { width:auto; flex:1; height:44px; }
  .discover-filter-bar { flex-wrap:wrap; gap:5px; margin-bottom:15px; }
  .discover-type-filters { width:100%; gap:0; }
  .discover-type-filters button { padding:0 10px; }
  .discover-grid-actions { position:static; width:100%; justify-content:flex-end; }
  .section-four-row,.section-infinite-grid { grid-template-columns:repeat(2,minmax(0,1fr)); }
}
</style>
