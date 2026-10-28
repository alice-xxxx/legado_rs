<script setup lang="ts">
import { Preloader as KPreloader, Button as KButton } from "konsta/vue";
defineOptions({ name: "HomePage" });

/* 根据栏目配置加载发现内容，并复用搜索页的书目操作。 */
import { computed, onActivated, onBeforeUnmount, onDeactivated, onMounted, reactive, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { listDiscoveryBooks } from "../../api/discovery";
import type { LoginUpdate } from "../../api/sources";

import { bookContentType } from "../../api/bookContentType";
import { sources } from "../sources/sourcesStore";
import { settings as sharedSettings } from "../settings/settingsStore";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import type {
  AppBootstrap,
  SearchBookResult,
  SearchResource,
 } from "../../api/types";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import router, { routeNames, settingsRouteNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import PrimaryNavigation from "../../ui/PrimaryNavigation.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import SelectMenu from "../../ui/SelectMenu.vue";

function openSearch(): void {
  void router.push({ name: routeNames.search });
}

function manageHome(): void {
  void router.push({ name: settingsRouteNames["home-config"] });
}

// 栏目内容按 ID 缓存；书源元数据只用于判断内容类型。
const homeConfig = computed(() => sharedSettings.value.home);
const sectionResources = ref<Record<string, SearchResource>>({});
const loadError = ref("");
let unlisteners: UnlistenFn[] = [];
// 记录每个栏目当前正在执行的请求，避免重复加载。
const sectionRequests = reactive(new Map<string, { sourceId: string; categoryId: string; token: symbol }>());

const activeTabId = ref("");
const activeType = ref<"all" | "novel" | "comic" | "video" | "audio">("all");
const typeFilters = [
  { id: "all", title: "全部" },
  { id: "novel", title: "文本" },
  { id: "comic", title: "图片" },
  { id: "video", title: "视频" },
  { id: "audio", title: "音频" },
] as const;
const orderedTabs = computed(() => [...homeConfig.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
// 合并当前标签页各栏目已加载的书目，按书源和书籍地址去重，最多显示 60 条。
const gridResults = computed(() => {
  const seen = new Set<string>();
  const items = orderedSections.value.flatMap((section) => sectionResources.value[section.id]?.results ?? []);
  return items.filter((item) => {
    const key = [item.sourceId, item.title, item.author ?? "", item.contentType ?? ""].join("\0");
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).slice(0, 60);
});

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// 暂存被点击的书目并打开详情页；导航失败时清除暂存请求。
async function openSearchResult(result: SearchBookResult): Promise<void> {
  if (shelfBatchRecoveryRequired.value) return;
  const requestId = requestBookDetailOpen({ kind: "search-result", result });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

// 使用 Rust 发布的内容类型。
function contentType(result: SearchBookResult): "novel" | "comic" | "video" | "audio" {
  const source = sources.value.find((item) => item.id === result.sourceId);
  const type = bookContentType(result) ?? bookContentType(source) ?? "text";
  return type === "image" ? "comic" : type === "audio" || type === "video" ? type : "novel";
}

const visibleResults = computed(() => gridResults.value.filter(
  (item) => activeType.value === "all" || contentType(item) === activeType.value,
));
const gridLoading = computed(() => orderedSections.value.some((section) => sectionRequests.has(section.id)));
const gridError = computed(() => loadError.value || orderedSections.value.flatMap(section => sectionResources.value[section.id]?.errors ?? []).map(error => error.message).join("；"));
const pullRefreshThreshold = 72;
const pullDistance = ref(0);
const pullRefreshing = ref(false);
const pullReady = computed(() => pullDistance.value >= pullRefreshThreshold);
const pullSource = ref<"touch" | "wheel" | null>(null);
let pullStartY: number | null = null;
let pullStartX: number | null = null;
let wheelPullResetTimer: ReturnType<typeof setTimeout> | null = null;
let pageEntryRevision = 0;
let emptyEntryRefreshInFlight = false;

// 只加载当前配置中的栏目，并忽略恢复或改配置后过期的响应。
async function loadSection(sectionId: string, sourceId: string, categoryId: string): Promise<void> {
  const belongsToCurrentConfig = () => homeConfig.value.tabs.some((tab) => tab.sections.some(
    (section) => section.id === sectionId && section.sourceId === sourceId && section.categoryId === categoryId,
  ));
  if (!belongsToCurrentConfig()) return;
  const ongoing = sectionRequests.get(sectionId);
  if (ongoing?.sourceId === sourceId && ongoing.categoryId === categoryId) return;
  const token = Symbol(sectionId);
  sectionRequests.set(sectionId, { sourceId, categoryId, token });
  const isCurrent = () => sectionRequests.get(sectionId)?.token === token
    && belongsToCurrentConfig();
  try {
    const response = await listDiscoveryBooks(sourceId, categoryId, 1);
    const resource = response.search;
    if (!resource) throw new Error("Discovery results missing");
    if (!isCurrent()) return;
    sectionResources.value = { ...sectionResources.value, [sectionId]: resource };
  } catch (error) {
    if (!isCurrent()) return;
    sectionResources.value = {
      ...sectionResources.value,
      [sectionId]: {
        keyword: "",
        page: 1,
        results: [],
        errors: [{ message: errorText(error) }],
      },
    };
  } finally {
    if (sectionRequests.get(sectionId)?.token === token) {
      sectionRequests.delete(sectionId);
    }
  }
}

// Changing a configured source invalidates only that section's loaded content.
watch(() => homeConfig.value.tabs.flatMap(tab => tab.sections.map(section =>
  JSON.stringify([section.id, section.sourceId, section.categoryId]))), (next, previous = []) => {
  const unchanged = new Set(next.filter(key => previous.includes(key))
    .map(key => (JSON.parse(key) as string[])[0]));
  sectionResources.value = Object.fromEntries(Object.entries(sectionResources.value)
    .filter(([id]) => unchanged.has(id)));
  for (const id of sectionRequests.keys()) {
    if (!unchanged.has(id)) sectionRequests.delete(id);
  }
});
watch(() => orderedSections.value.map(section =>
  JSON.stringify([section.id, section.sourceId, section.categoryId])), () => {
  for (const section of orderedSections.value) {
    if (!sectionResources.value[section.id]) void loadSection(section.id, section.sourceId, section.categoryId);
  }
}, { immediate: true });

// 应用恢复备份或页面销毁时，让未完成的旧请求失效并清空页面缓存。
function resetForRestore(): void {
  sectionRequests.clear();
  sectionResources.value = {};
}

// 后端通知应用状态已恢复后，重新读取书源、栏目配置和书架。
function handleAppStateUpdated(): void {
  resetForRestore();
}
function selectTab(id: string): void {
  activeTabId.value = id;
  const tab = orderedTabs.value.find((item) => item.id === id);
  for (const section of tab?.sections ?? []) {
    if (!sectionResources.value[section.id]) {
      void loadSection(section.id, section.sourceId, section.categoryId);
    }
  }
}

// 手动或空内容自动刷新时重新加载当前标签页的栏目；没有栏目时先读取最新配置。
async function refreshGrid(): Promise<void> {
  await Promise.all(orderedSections.value.map((section) =>
    loadSection(section.id, section.sourceId, section.categoryId)));
}

// 首页由 KeepAlive 缓存；进入时等已有加载结束，再按内容状态决定是否补刷新。
async function refreshEmptyDiscoveryOnEntry(entryRevision: number): Promise<void> {
  if (emptyEntryRefreshInFlight) return;
  if (gridLoading.value) return;
  if (entryRevision !== pageEntryRevision
    || gridResults.value.length > 0
    || gridLoading.value
    || emptyEntryRefreshInFlight) return;

  emptyEntryRefreshInFlight = true;
  try {
    await refreshGrid();
  } finally {
    emptyEntryRefreshInFlight = false;
  }
}

onActivated(() => {
  const entryRevision = ++pageEntryRevision;
  void refreshEmptyDiscoveryOnEntry(entryRevision);
});

onDeactivated(() => {
  // 离开页面时让尚未开始的空内容检查失效，返回时会重新判断。
  pageEntryRevision += 1;
});

function clearWheelPullResetTimer(): void {
  if (wheelPullResetTimer !== null) clearTimeout(wheelPullResetTimer);
  wheelPullResetTimer = null;
}

function resetPullState(): void {
  pullStartY = null;
  pullStartX = null;
  pullDistance.value = 0;
  pullSource.value = null;
}

// 页面位于顶部、没有加载任务且单指触摸时开始识别下拉手势；空页面也允许刷新。
function handlePullStart(event: TouchEvent): void {
  clearWheelPullResetTimer();
  resetPullState();
  const scroller = event.currentTarget as HTMLElement;
  if (pullRefreshing.value || gridLoading.value || event.touches.length !== 1 || scroller.scrollTop > 0) return;
  pullStartY = event.touches[0].clientY;
  pullStartX = event.touches[0].clientX;
  pullSource.value = "touch";
}

// 只响应向下的纵向拖动；横向滑动不触发刷新，提示距离最多为 104px。
function handlePullMove(event: TouchEvent): void {
  if (pullStartY === null || pullStartX === null || event.touches.length !== 1) return;
  const scroller = event.currentTarget as HTMLElement;
  if (scroller.scrollTop > 0) {
    resetPullState();
    return;
  }
  const touch = event.touches[0];
  const deltaY = touch.clientY - pullStartY;
  if (Math.abs(touch.clientX - pullStartX) > Math.abs(deltaY)) {
    resetPullState();
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
    if (pullSource.value === "wheel") resetPullState();
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
      resetPullState();
    }
  }, 260);
}

// 两种输入共用同一刷新动作；触摸松手或滚轮停止输入后，达到阈值才刷新。
async function startGridRefresh(): Promise<void> {
  if (pullRefreshing.value || gridLoading.value) {
    resetPullState();
    return;
  }
  clearWheelPullResetTimer();
  pullRefreshing.value = true;
  pullDistance.value = pullRefreshThreshold;
  try {
    await refreshGrid();
  } finally {
    pullRefreshing.value = false;
    resetPullState();
  }
}

// 触摸松手超过 72px 才刷新；没达到阈值就收起提示条。
async function handlePullEnd(): Promise<void> {
  const ready = pullReady.value;
  pullStartY = null;
  pullStartX = null;
  if (!ready) {
    resetPullState();
    return;
  }
  await startGridRefresh();
}

// 系统取消触摸手势时，复位提示条和手势坐标。
function handlePullCancel(): void {
  pullStartY = null;
  pullStartX = null;
  if (!pullRefreshing.value) {
    resetPullState();
  }
}

// 配置更新后，如果当前选中的标签被删除，就自动选中第一个标签。
watch(orderedTabs, (tabs) => {
  if (!tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = tabs[0]?.id ?? "";
}, { immediate: true });

// 读取书源的轻量元数据，并移除不需要放在页面状态里的完整规则 JSON。
// 订阅数据更新和恢复事件，再读取启动数据。
onMounted(async () => {
  try {
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", () => {
      handleAppStateUpdated();
    }));
    // 登录规则要求刷新发现内容时，只重载该源对应的栏目。
    unlisteners.push(await listen<LoginUpdate>("login", ({ payload }) => {
      if (!payload.refreshTargets?.includes("explore")) return;
      for (const section of homeConfig.value.tabs.flatMap(tab => tab.sections)) {
        if (section.sourceId !== payload.sourceId) continue;
        delete sectionResources.value[section.id];
        sectionRequests.delete(section.id);
        void loadSection(section.id, section.sourceId, section.categoryId);
      }
    }));
  } catch (error) {
    loadError.value = errorText(error);
  }
});

// 页面销毁时取消事件订阅，并使未完成的异步请求失效。
onBeforeUnmount(() => {
  clearWheelPullResetTimer();
  resetForRestore();
  for (const unlisten of unlisteners) unlisten();
  unlisteners = [];
});
</script>

<template>
  <!-- 下拉刷新手势监听在滚动区上。 -->
  <div class="home-page-layout">
    <PrimaryNavigation />
    <main class="home-page-main" @touchstart.passive="handlePullStart" @touchmove.passive="handlePullMove"
      @touchend="handlePullEnd" @touchcancel="handlePullCancel" @wheel.passive="handleWheelPull">
      <SubpageHeader title="发现" :back="false" large>
        <template #actions>
          <KButton rounded inline clear class="size-11 p-0" type="button" aria-label="搜索" @click="openSearch"><PrototypeIcon name="search" /></KButton>
          <KButton rounded inline clear class="size-11 p-0" type="button" aria-label="管理栏目" @click="manageHome"><PrototypeIcon name="tune" /></KButton>
        </template>
      </SubpageHeader>
      <section class="home-discovery ui-page-content" :class="{ 'home-discovery-empty': !visibleResults.length }">
        <!-- 标签靠左，分类筛选和操作按钮靠右。 -->
        <div class="discover-filter-bar">
          <nav v-if="orderedTabs.length" class="home-tabs" aria-label="发现页面">
            <KButton rounded :tonal="activeTab?.id === tab.id" :clear="!(activeTab?.id === tab.id)" inline class="gap-2" v-for="tab in orderedTabs" :key="tab.id"
              :class="{ active: activeTab?.id === tab.id }"
              @click="selectTab(tab.id)">{{ tab.title }}</KButton>
          </nav>
          <div class="discover-controls">
            <SelectMenu id="discover-type-button" v-model="activeType" label="内容类型"
              :options="typeFilters.map(filter => ({ value: filter.id, label: filter.title }))" />
          </div>
        </div>

        <!-- 下拉时显示距离，松手刷新时显示进度。 -->
        <div v-if="pullDistance > 0 || pullRefreshing" class="home-pull-refresh"
          :style="{ height: `${pullRefreshing ? pullRefreshThreshold : pullDistance}px` }"
          role="status" aria-live="polite">
          <span class="home-pull-refresh-icon" :class="{ ready: pullReady, refreshing: pullRefreshing }">
            <PrototypeIcon name="refresh" />
          </span>
          <span>{{ pullRefreshing ? '正在刷新…' : pullReady ? '松开刷新' : pullSource === 'wheel' ? '继续向上滚动下拉刷新' : '下拉刷新' }}</span>
        </div>

        <!-- 封面流展示当前栏目内容，也显示加载、空内容和错误状态。 -->
        <div v-if="visibleResults.length" class="discovery-content-grid">
          <article v-for="(result, index) in visibleResults" :key="`${result.sourceId}:${result.resultId}`"
            class="discovery-cover-card">
            <button type="button" class="discovery-cover-action" @click="openSearchResult(result)">
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
        <div v-else-if="gridLoading" class="page-empty-state discover-grid-state" role="status">
          <KPreloader />
          <span>正在从源加载发现内容…</span>
        </div>
        <div v-else class="page-empty-state discover-grid-state">
          <span class="empty-state-glyph"><PrototypeIcon :name="gridError ? 'refresh' : 'compass'" /></span>
          <strong>{{ gridError ? '发现内容暂时无法加载' : activeType !== 'all' && gridResults.length ? '暂无匹配内容' : '发现你的下一本好书' }}</strong>
          <p>{{ gridError || (orderedSections.length ? '试试其他类型，或刷新当前栏目。' : '添加内容来源并配置栏目，开始探索。') }}</p>
          <KButton rounded inline @click="orderedSections.length ? refreshGrid() : manageHome()">{{ orderedSections.length ? '重新加载' : '配置发现栏目' }}</KButton>
        </div>
        <p v-if="gridError && visibleResults.length" class="status-notice is-error" role="alert">{{ gridError }}</p>
      </section>
    </main>
  </div>
</template>

<style scoped>
@layer components {
/* 页面滚动容器；触顶时限制浏览器自己的回弹，由页面处理下拉刷新。 */
.home-page-layout { display:flex; width:100%; height:100dvh; min-height:0; overflow:hidden; background:var(--app-background); }
.home-page-main { flex:1; min-width:0; min-height:0; overflow-x:hidden; overflow-y:auto; overscroll-behavior-y:contain; padding:0 0 var(--safe-bottom); }
/* 下拉时显示的提示条和图标状态。 */
.home-pull-refresh { display:flex; align-items:center; justify-content:center; gap:8px; overflow:hidden; color:var(--app-muted); font-size:var(--ui-font-caption); transition:height .12s ease; }
.home-pull-refresh-icon { display:grid; width:16px; height:16px; place-items:center; }
.home-pull-refresh-icon svg { width:16px; height:16px; transition:transform .15s ease; }
.home-pull-refresh-icon.ready svg { transform:rotate(180deg); }
.home-pull-refresh-icon.refreshing svg { animation:home-spin .8s linear infinite; }
/* 标签靠左，分类筛选和操作按钮靠右排列。 */
.home-discovery { min-width:0; }
.home-discovery-empty { display:flex; flex-direction:column; min-height:calc(100% - 132px); }
.home-discovery-empty .discover-filter-bar { flex:none; }
.discover-filter-bar { display:flex; justify-content:space-between; align-items:center; gap:20px; min-height:50px; margin-bottom:20px; padding:0 0 12px; border-bottom:1px solid var(--app-line); }
.home-tabs { display:flex; min-width:0; align-items:center; gap:4px; overflow-x:auto; scrollbar-width:none; }
.home-tabs::-webkit-scrollbar { display:none; }
.home-tabs button { flex:none; }
.home-tabs button.active { color:var(--app-text); font-weight:600; }
.discover-controls { display:flex; flex:none; align-items:center; gap:12px; }
.discover-icon-button { display:grid; width:30px; height:30px; padding:0; place-items:center; }
.discover-icon-button svg { width:18px; height:18px; }
.discovery-content-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(var(--ui-cover-min-width),1fr)); gap:var(--ui-grid-gap); }
.discovery-cover-card { min-width:0; }
.discovery-cover-action { display:block; width:100%; padding:0; border:0; background:transparent; text-align:left; cursor:pointer; }
.discovery-cover-art { position:relative; display:block; width:100%; overflow:hidden; aspect-ratio:3/4; border-radius:13px; background:linear-gradient(140deg,#466f6d,#1b3235);  }
.discovery-cover-art img { width:100%; height:100%; object-fit:cover; display:block; }
.discovery-cover-placeholder { position:absolute; inset:0; overflow:hidden; }
.discovery-cover-placeholder::before { content:""; position:absolute; width:76%; height:72%; right:-28%; top:15%; border-radius:50%; background:#fff2; }
.discovery-cover-placeholder::after { content:""; position:absolute; left:-25%; right:-20%; bottom:-37%; height:65%; border-radius:50%; background:#0003; }
/* 封面卡片文字、加载/空状态和错误提示。 */
.discovery-cover-copy { display:flex; flex-direction:column; min-width:0; padding:8px 1px 0; gap:3px; }
.discovery-cover-copy strong { overflow:hidden; color:var(--app-text); font-size:var(--ui-font-body); white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy small { overflow:hidden; color:var(--app-muted); font-size:var(--ui-font-caption); white-space:nowrap; text-overflow:ellipsis; }
.discovery-cover-copy>span { overflow:hidden; color:var(--app-subtle); font-size:var(--ui-font-caption); white-space:nowrap; text-overflow:ellipsis; }
.discover-grid-state { min-height:230px; display:flex; align-items:center; justify-content:center; gap:12px; padding:24px; color:var(--app-muted); }
.home-discovery-empty .discover-grid-state { flex:1; min-height:160px; text-align:center; }
@keyframes home-spin { to {transform:rotate(360deg)} }
/* 屏幕变窄时减少卡片列数并调整工具栏布局。 */
@media(max-width:860px) {
  .home-page-main { padding:0 0 calc(var(--ui-nav-height) + var(--safe-bottom)); }
}
@media(max-width:640px) {
  .discover-filter-bar { gap:8px; margin-bottom:16px; padding-bottom:10px; }
  .home-tabs { gap:8px; }
  .discover-controls { gap:5px; }
}
}
</style>
