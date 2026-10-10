<script setup lang="ts">
defineOptions({ name: "HomePage" });

/*
 * 首页负责读取栏目配置、加载栏目内容，并显示搜索入口和内容卡片。
 * 内容来自“管理栏目”里配置的书源和分类
 * ref 保存会变化的状态，computed 根据这些状态自动计算页面数据。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { addBook } from "../../api/books";
import { getHomeConfig, listDiscoveryBooks } from "../../api/discovery";
import { readResource } from "../../api/resources";
import { appBootstrap } from "../../api/core";
import { shelfBatchRecoveryMessage, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import type {
  AppBootstrap,
  BookResource,
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

// 顶部两个入口分别跳到搜索页和首页栏目配置页。
function openSearch(): void {
  void router.push({ name: routeNames.search });
}

function manageHome(): void {
  void router.push({ name: settingsRouteNames["home-config"] });
}

// homeConfig 保存标签页/栏目的设置；sectionResources 按栏目 ID 缓存加载结果。
// sources 只用于判断内容类型，不用于自动生成推荐内容。
const homeConfig = ref<HomeConfigDocument>({
  schemaVersion: 1,
  tabs: [{ id: "tab-home", title: "主页", sortOrder: 0, sections: [] }],
});
const sectionResources = ref<Record<string, SearchResource>>({});
const loadingSectionIds = ref<string[]>([]);
const sources = ref<SourceMetadata[]>([]);
const configLoading = ref(true);
const loadError = ref("");
const restoreRevision = ref(0);
// 请求版本号用于忽略过期的页面、配置或书源响应。
let pageRevision = 0;
let configRequestRevision = 0;
let sourceRequestRevision = 0;
let unlisteners: UnlistenFn[] = [];
// 记录每个栏目当前正在执行的请求，避免重复加载。
const sectionRequests = new Map<string, { sourceId: string; categoryId: string; token: symbol }>();

// busy 表示首页配置正在读取；其余状态决定当前标签、内容类型和展示视图。
const busy = computed(() => configLoading.value);
const activeTabId = ref("");
const activeType = ref<"all" | "novel" | "comic" | "video" | "audio">("all");
const typeFilters = [
  { id: "all", title: "全部" },
  { id: "novel", title: "小说" },
  { id: "comic", title: "漫画" },
  { id: "video", title: "视频" },
  { id: "audio", title: "音频" },
] as const;
// 以下计算值按排序后的配置找到当前标签页和它的栏目。
const orderedTabs = computed(() => [...homeConfig.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
// 合并当前标签页各栏目已加载的书目，按书源和书籍地址去重，最多显示 60 条。
const gridResults = computed(() => {
  const seen = new Set<string>();
  const items = orderedSections.value.flatMap((section) => sectionResources.value[section.id]?.results ?? []);
  return items.filter((item) => {
    const key = item.sourceId + ":" + (item.bookUrl ?? item.resultId);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).slice(0, 60);
});

// 把 JavaScript 异常统一转换成可显示的文本。
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// 打开书籍详情时会用到当前书架和搜索结果操作状态。
const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const detailSearchResults = ref<SearchBookResult[]>([]);
let shelfReadRevision = 0;
async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  // 多次读取时只接受最后一次结果，避免较慢的旧请求覆盖新书架。
  const requestRevision = ++shelfReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  const nextShelf = await readResource<ShelfResource>(descriptor);
  if (currentRestoreRevision !== restoreRevision.value || requestRevision !== shelfReadRevision) return;
  shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
}

// 复用搜索页的书目操作状态和“加入书架”动作，并把它们交给详情页。
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

// 暂存被点击的书目并打开详情页；导航失败时清除暂存请求。
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

// 优先采用结果自带的类型；没有时根据书源的媒体类型或分组推断。
function contentType(result: SearchBookResult): "novel" | "comic" | "video" | "audio" {
  const declared = [result.contentType, result.mediaType, result.type]
    .find((value) => typeof value === "string");
  if (typeof declared === "string") {
    if (/^(comic|manga|漫画)$/i.test(declared)) return "comic";
    if (/^(video|视频)$/i.test(declared)) return "video";
    if (/^(audio|音频)$/i.test(declared)) return "audio";
    if (/^(novel|text|小说)$/i.test(declared)) return "novel";
  }
  const source = sources.value.find((item) => item.id === result.sourceId);
  if (source?.mediaType === "video" || source?.mediaType === "audio") return source.mediaType;
  if (/漫画|comic|manga/i.test(source?.group ?? "")) return "comic";
  return "novel";
}

const visibleResults = computed(() => gridResults.value.filter(
  (item) => activeType.value === "all" || contentType(item) === activeType.value,
));
const gridLoading = computed(() => busy.value
  || orderedSections.value.some((section) => loadingSectionIds.value.includes(section.id)));
const pullRefreshThreshold = 72;
const pullDistance = ref(0);
const pullRefreshing = ref(false);
const pullReady = computed(() => pullDistance.value >= pullRefreshThreshold);
const pullSource = ref<"touch" | "wheel" | null>(null);
let pullStartY: number | null = null;
let pullStartX: number | null = null;
let wheelPullResetTimer: ReturnType<typeof setTimeout> | null = null;

// 加载一个栏目：先确认它仍属于当前配置，再避免重复请求。
// 请求返回时检查配置和版本，防止过期响应覆盖切换配置/恢复数据后的状态。
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

// 读取并应用首页配置。栏目未变时保留缓存，变化时清除旧结果；随后顺序加载
// 第一个标签页中的全部栏目。
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

  void (async () => {
    for (const section of tabs[0]?.sections ?? []) {
      if (!isCurrent()) return;
      await loadSection(section.id, section.sourceId, section.categoryId);
    }
  })();
}

// 应用恢复备份或页面销毁时，让未完成的旧请求失效并清空页面缓存。
function resetForRestore(): void {
  pageRevision += 1;
  configRequestRevision += 1;
  sourceRequestRevision += 1;
  homeConfig.value = { schemaVersion: 1, tabs: [] };
  sectionRequests.clear();
  sectionResources.value = {};
  loadingSectionIds.value = [];
}

// 后端通知应用状态已恢复后，重新读取书源、栏目配置和书架。
async function handleAppStateUpdated(bootstrap: AppBootstrap): Promise<void> {
  resetForRestore();
  restoreRevision.value += 1;
  homeSearchResultActions.resetSearchResultState();
  detailSearchResults.value = [];
  await Promise.all([refreshSources(bootstrap.sources), refreshConfig(), refreshShelf(bootstrap.shelf)]);
}
// 切换标签页时加载其中尚未缓存的栏目。
function selectTab(id: string): void {
  activeTabId.value = id;
  const tab = orderedTabs.value.find((item) => item.id === id);
  for (const section of tab?.sections ?? []) {
    if (!Object.prototype.hasOwnProperty.call(sectionResources.value, section.id)) {
      void loadSection(section.id, section.sourceId, section.categoryId);
    }
  }
}

// 下拉刷新后并发重新加载当前标签页的全部栏目；当前没有栏目时重新读取栏目配置。
async function refreshGrid(): Promise<void> {
  if (!orderedSections.value.length) await refreshConfig();
  await Promise.all(orderedSections.value.map((section) =>
    loadSection(section.id, section.sourceId, section.categoryId)));
}

function clearWheelPullResetTimer(): void {
  if (wheelPullResetTimer !== null) clearTimeout(wheelPullResetTimer);
  wheelPullResetTimer = null;
}

// 页面位于顶部、没有加载任务且单指触摸时开始识别下拉手势；空页面也允许刷新。
function handlePullStart(event: TouchEvent): void {
  clearWheelPullResetTimer();
  pullStartY = null;
  pullStartX = null;
  pullDistance.value = 0;
  pullSource.value = null;
  if (pullRefreshing.value || gridLoading.value || event.touches.length !== 1) return;
  const scroller = event.currentTarget as HTMLElement;
  if (scroller.scrollTop > 0) return;
  pullStartY = event.touches[0].clientY;
  pullStartX = event.touches[0].clientX;
  pullSource.value = "touch";
}

// 只响应向下的纵向拖动；横向滑动不触发刷新，提示距离最多为 104px。
function handlePullMove(event: TouchEvent): void {
  if (pullStartY === null || pullStartX === null || event.touches.length !== 1) return;
  const scroller = event.currentTarget as HTMLElement;
  if (scroller.scrollTop > 0) {
    pullStartY = null;
    pullStartX = null;
    pullDistance.value = 0;
    return;
  }
  const touch = event.touches[0];
  const deltaY = touch.clientY - pullStartY;
  if (Math.abs(touch.clientX - pullStartX) > Math.abs(deltaY)) {
    pullStartY = null;
    pullStartX = null;
    pullDistance.value = 0;
    return;
  }
  pullDistance.value = Math.min(Math.max(deltaY, 0), 104);
}

// PC 没有触摸拖动：在页面顶部继续向上滚动，累计滚轮/触控板位移来触发同一刷新。
function handleWheelPull(event: WheelEvent): void {
  const scroller = event.currentTarget as HTMLElement;
  if (pullRefreshing.value) return;
  if (event.deltaY >= 0 || scroller.scrollTop > 0 || gridLoading.value) {
    clearWheelPullResetTimer();
    if (pullSource.value === "wheel") {
      pullDistance.value = 0;
      pullSource.value = null;
    }
    return;
  }

  const multiplier = event.deltaMode === WheelEvent.DOM_DELTA_LINE
    ? 16
    : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? scroller.clientHeight : 1;
  const distance = Math.abs(event.deltaY * multiplier);
  pullSource.value = "wheel";
  pullDistance.value = Math.min(pullDistance.value + distance, 104);
  clearWheelPullResetTimer();
  // PC 没有触摸松手事件，用滚轮/触控板停止输入一小段时间作为“松手”。
  wheelPullResetTimer = setTimeout(() => {
    wheelPullResetTimer = null;
    if (pullDistance.value >= pullRefreshThreshold && pullSource.value === "wheel") {
      void startGridRefresh();
    } else {
      pullDistance.value = 0;
      pullSource.value = null;
    }
  }, 260);
}

// 两种输入共用同一刷新动作；触摸松手或滚轮停止输入后，达到阈值才刷新。
async function startGridRefresh(): Promise<void> {
  if (pullRefreshing.value || gridLoading.value) {
    pullDistance.value = 0;
    pullSource.value = null;
    return;
  }
  clearWheelPullResetTimer();
  pullRefreshing.value = true;
  pullDistance.value = pullRefreshThreshold;
  try {
    await refreshGrid();
  } finally {
    pullRefreshing.value = false;
    pullDistance.value = 0;
    pullSource.value = null;
  }
}

// 触摸松手超过 72px 才刷新；没达到阈值就收起提示条。
async function handlePullEnd(): Promise<void> {
  pullStartY = null;
  pullStartX = null;
  if (pullDistance.value < pullRefreshThreshold) {
    pullDistance.value = 0;
    pullSource.value = null;
    return;
  }
  await startGridRefresh();
}

// 系统取消触摸手势时，复位提示条和手势坐标。
function handlePullCancel(): void {
  pullStartY = null;
  pullStartX = null;
  if (!pullRefreshing.value) {
    pullDistance.value = 0;
    pullSource.value = null;
  }
}

// 配置更新后，如果当前选中的标签被删除，就自动选中第一个标签。
watch(orderedTabs, (tabs) => {
  if (!tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = tabs[0]?.id ?? "";
}, { immediate: true });

// 读取栏目配置时显示加载状态；出错时保留错误信息供页面展示。
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

// 读取书源的轻量元数据，并移除不需要放在页面状态里的完整规则 JSON。
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

// 页面创建后订阅配置/书源/书架/恢复事件，然后读取当前应用数据。
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

// 页面销毁时取消事件订阅，并使未完成的异步请求失效。
onBeforeUnmount(() => {
  clearWheelPullResetTimer();
  pageRevision += 1;
  restoreRevision.value += 1;
  resetForRestore();
  for (const unlisten of unlisteners) unlisten();
  unlisteners = [];
});
</script>

<template>
  <!-- 模板语法：v-if 控制显示，v-for 循环生成列表，: 绑定动态属性，@ 绑定事件。 -->
  <!-- 页面由全局导航和可滚动的首页内容区组成；下拉手势监听在滚动区上。 -->
  <div class="home-page-layout">
    <PrimaryNavigation />
  <main class="home-page-main" @touchstart.passive="handlePullStart" @touchmove.passive="handlePullMove"
      @touchend="handlePullEnd" @touchcancel="handlePullCancel" @wheel.passive="handleWheelPull">
      <section class="home-page">
    <!-- 搜索入口：点击后进入完整搜索页面。 -->
    <div class="discover-command">
      <button class="discover-search-launch" type="button" @click="openSearch">
        <span class="discover-search-icon"><PrototypeIcon name="search"/></span>
        <span>搜索</span>
      </button>
    </div>
    <section class="home-discovery">
      <!-- 类型筛选和栏目配置入口。 -->
      <div class="discover-filter-bar">
        <div class="discover-type-filters">
          <button v-for="filter in typeFilters" :key="filter.id" type="button"
            :class="{ active: activeType === filter.id }"
            @click="activeType = filter.id">{{ filter.title }}</button>
        </div>
        <div class="discover-grid-actions">
          <button type="button" class="discover-utility-button"
            :disabled="busy" @click="manageHome">管理</button>
        </div>
      </div>
      <!-- 下拉提示紧跟类型筛选行；拖动时显示距离，松手刷新时显示进度。 -->
      <div v-if="pullDistance > 0 || pullRefreshing" class="home-pull-refresh"
        :style="{ height: `${pullRefreshing ? pullRefreshThreshold : pullDistance}px` }"
        role="status" aria-live="polite">
        <span class="home-pull-refresh-icon" :class="{ ready: pullReady, refreshing: pullRefreshing }">
          <PrototypeIcon name="refresh" />
        </span>
        <span>{{ pullRefreshing ? '正在刷新…' : pullReady ? '松开刷新' : pullSource === 'wheel' ? '继续向上滚动下拉刷新' : '下拉刷新' }}</span>
      </div>
      <div v-if="orderedTabs.length > 1" class="home-tabs">
        <button v-for="tab in orderedTabs" :key="tab.id"

          :class="{ active: activeTab?.id === tab.id }" @click="selectTab(tab.id)">{{ tab.title }}</button>
      </div>
      <!-- 封面流：展示合并后的栏目内容，或加载中、空内容和错误状态。 -->
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
        <span class="mini-loader"></span><span>正在从源加载发现内容…</span>
      </div>
      <div v-else class="discover-grid-state discover-grid-empty">
        <strong>{{ activeType !== 'all' && gridResults.length ? '当前类型还没有内容' : '暂无可显示的发现内容' }}</strong>
      </div>
      <p v-if="loadError" class="discover-grid-warning">{{ loadError }}</p>
    </section>
      </section>
    </main>
  </div>
</template>

<style scoped>
/* 首页内容区按原型展示为封面流。 */
/* 页面滚动容器；触顶时限制浏览器自己的回弹，由页面处理下拉刷新。 */
.home-page-layout { display:flex; width:100%; height:100dvh; min-height:0; overflow:hidden; padding-top:var(--safe-top); background:var(--app-background); }
.home-page-main { flex:1; min-width:0; min-height:0; overflow-x:hidden; overflow-y:auto; overscroll-behavior-y:contain; padding:0 clamp(20px,3.6vw,56px) var(--safe-bottom); }
/* 下拉时显示的提示条和图标状态。 */
.home-pull-refresh { display:flex; align-items:center; justify-content:center; gap:8px; overflow:hidden; color:var(--app-muted); font-size:12px; transition:height .12s ease; }
.home-pull-refresh-icon { display:grid; width:16px; height:16px; place-items:center; }
.home-pull-refresh-icon svg { width:16px; height:16px; transition:transform .15s ease; }
.home-pull-refresh-icon.ready svg { transform:rotate(180deg); }
.home-pull-refresh-icon.refreshing svg { animation:home-spin .8s linear infinite; }
/* 搜索框和发现页工具栏。 */
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
/* 封面卡片文字、加载/空状态和错误提示。 */
.discovery-cover-copy { display:flex; flex-direction:column; min-width:0; padding:8px 1px 0; gap:3px; }
.discovery-cover-copy strong { overflow:hidden; color:#24252d; font-size:13px; white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy small { overflow:hidden; color:#838896; font-size:11px; white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy>span { overflow:hidden; color:#959aa5; font-size:10px; white-space:nowrap; text-overflow:ellipsis; }
.discover-grid-state { min-height:230px; display:flex; align-items:center; justify-content:center; gap:12px; padding:24px; border:1px dashed #e2e4eb; border-radius:15px; color:#828896; background:#ffffff7a; }
.discover-grid-empty { min-height:0; padding:24px 0; border:0; background:transparent; }
.discover-grid-empty strong { font-size:15px; color:#40444f; }
.discover-grid-warning { font-size:11px; color:#af6555; overflow-wrap:anywhere; }
/* 多个标签页时显示的切换栏。 */
.home-tabs { display:flex; justify-content:center; gap:5px; margin:0 0 20px; overflow-x:auto; }
.home-tabs button { padding:8px 14px; border:0; border-radius:999px; background:transparent; color:#787e89; cursor:pointer; }
.home-tabs button.active { background:var(--app-accent-soft); color:var(--app-accent); font-weight:700; }
.mini-loader { display:inline-block; width:16px; height:16px; border:2px solid #ddd8fb; border-top-color:var(--app-accent); border-radius:50%; animation:home-spin .8s linear infinite; }
@keyframes home-spin { to {transform:rotate(360deg)} }
/* 屏幕变窄时减少卡片列数并调整工具栏布局。 */
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
}
</style>
