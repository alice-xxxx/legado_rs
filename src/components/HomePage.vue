<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type {
  AppTask,
  HomeConfigDocument,
  ReadingHistoryResource,
  SearchBookResult,
  SearchResource,
  ShelfResource,
  SourceMetadata,
} from "../api/app";

const props = withDefaults(
  defineProps<{
    shelf: ShelfResource;
    readingHistory: ReadingHistoryResource;
    sources: SourceMetadata[];
    homeConfig: HomeConfigDocument;
    sectionResults?: Record<string, SearchResource | undefined>;
    loadingSectionIds?: string[];
    tasks?: AppTask[];
    busy?: boolean;
  }>(),
  { sectionResults: () => ({}), loadingSectionIds: () => [], tasks: () => [], busy: false },
);

const emit = defineEmits<{
  continueReading: [bookId: string];
  openHomeSection: [sectionId: string, sourceId: string, categoryId: string];
  openResult: [resultId: string];
  openShelf: [];
  openDiscovery: [];
  openHistory: [];
  openTasks: [];
  openSources: [];
  manageHome: [];
}>();

const activeTabId = ref(props.homeConfig.tabs[0]?.id ?? "");
const orderedTabs = computed(() => [...props.homeConfig.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));

watch(orderedTabs, (tabs) => {
  if (!tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = tabs[0]?.id ?? "";
}, { immediate: true });

const recentBooks = computed(() =>
  [...props.shelf.books]
    .filter((book) => (book.progress?.updatedAtMs ?? 0) > 0)
    .sort((left, right) => (right.progress?.updatedAtMs ?? 0) - (left.progress?.updatedAtMs ?? 0))
    .slice(0, 4),
);

const enabledSourceCount = computed(() => props.sources.filter((source) => source.enabled).length);
const activeTasks = computed(() =>
  props.tasks
    .filter((task) => ["queued", "running", "pausing", "paused", "cancelling"].includes(task.status))
    .sort((left, right) => right.updatedAtMs - left.updatedAtMs),
);

function formatDuration(value: number): string {
  const minutes = Math.floor(Math.max(0, value) / 60_000);
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} 小时 ${rest} 分钟` : `${hours} 小时`;
}

function readingPosition(book: ShelfResource["books"][number]): string {
  const chapter = Math.max(0, book.progress?.chapterIndex ?? 0) + 1;
  return `读到第 ${chapter} 章 · ${book.latestChapter || "目录已更新"}`;
}

function progressPercent(book: ShelfResource["books"][number]): number {
  if (!book.chapterCount) return 0;
  const chapter = Math.max(0, book.progress?.chapterIndex ?? 0) + 1;
  return Math.max(0, Math.min(100, (chapter / book.chapterCount) * 100));
}

function taskLabel(task: AppTask): string {
  if (task.kind === "search") return task.keyword ? `搜索“${task.keyword}”` : "书籍搜索";
  if (task.kind === "bookSourceCandidates") {
    const bookTitle = props.shelf.books.find((book) => book.id === task.bookId)?.title;
    return bookTitle ? `为《${bookTitle}》寻找其他书源` : "寻找其他书源";
  }
  if (task.kind === "chapterDownload") return "准备章节";
  if (task.kind === "refreshChapters") return "更新目录";
  if (task.kind === "checkNewChapters") return "检查新章节";
  return "后台任务";
}

function taskStatus(status: string): string {
  return ({
    queued: "等待中",
    running: "进行中",
    pausing: "正在暂停",
    paused: "已暂停",
    cancelling: "正在取消",
  } as Record<string, string>)[status] ?? "进行中";
}

function taskProgress(task: AppTask): number {
  if (task.total <= 0) return task.status === "running" ? 12 : 0;
  return Math.max(0, Math.min(100, Math.round((task.completed / task.total) * 100)));
}

function sectionResults(sectionId: string): SearchBookResult[] {
  return props.sectionResults[sectionId]?.results ?? [];
}

function sectionIsLoading(sectionId: string): boolean {
  return props.loadingSectionIds.includes(sectionId);
}

function sectionHasPartialError(sectionId: string): boolean {
  return (props.sectionResults[sectionId]?.errors.length ?? 0) > 0;
}
</script>

<template>
  <section class="home-page" aria-label="主页">
    <header class="home-intro">
      <div>
        <p class="home-eyebrow">你的阅读空间</p>
        <h2>继续上次阅读</h2>
        <p class="home-subtitle">
          {{ recentBooks.length ? "最近读过的书和阅读位置都在这里。" : "从书架挑一本书，开始今天的阅读。" }}
        </p>
      </div>
      <div class="home-actions">
        <button class="home-button home-button-secondary" :disabled="busy" @click="emit('openShelf')">打开书架</button>
        <button class="home-button home-button-primary" :disabled="busy" @click="emit('openDiscovery')">发现书籍</button>
      </div>
    </header>

    <section class="continue-section" aria-labelledby="recent-reading-title">
      <div class="section-heading">
        <div><p class="home-eyebrow">回到故事里</p><h3 id="recent-reading-title">最近阅读</h3></div>
        <button v-if="recentBooks.length" class="home-text-button" @click="emit('openShelf')">全部书籍 <span aria-hidden="true">→</span></button>
      </div>

      <div v-if="recentBooks.length" class="recent-books">
        <article v-for="book in recentBooks" :key="book.id" class="recent-book">
          <button class="recent-cover" :aria-label="`继续阅读《${book.title}》`" :disabled="busy" @click="emit('continueReading', book.id)">
            <img v-if="book.coverSrc" :src="book.coverSrc" alt="" loading="lazy" />
            <span v-else class="cover-placeholder">{{ book.title.slice(0, 1) || "书" }}</span>
          </button>
          <div class="recent-book-info">
            <strong class="recent-title">{{ book.title }}</strong>
            <span class="recent-author">{{ book.author || "作者未知" }}</span>
            <small>{{ readingPosition(book) }}</small>
            <div class="progress-track" role="progressbar" :aria-valuenow="Math.round(progressPercent(book))" aria-valuemin="0" aria-valuemax="100" :aria-label="`${book.title}阅读进度`">
              <span :style="{ width: `${progressPercent(book)}%` }"></span>
            </div>
          </div>
          <button class="home-button home-button-primary continue-button" :disabled="busy" @click="emit('continueReading', book.id)">继续阅读</button>
        </article>
      </div>
      <div v-else class="home-empty">
        <span class="empty-symbol" aria-hidden="true">⌑</span>
        <div><strong>还没有阅读进度</strong><p>添加一本书后，你的阅读位置会显示在这里。</p></div>
        <button class="home-button home-button-secondary" :disabled="busy" @click="emit('openShelf')">前往书架</button>
      </div>
    </section>

    <section class="home-stat-grid" aria-label="阅读概况">
      <article class="stat-card stat-reading">
        <span class="stat-icon" aria-hidden="true">◷</span>
        <div><small>累计阅读</small><strong>{{ formatDuration(readingHistory.totalDurationMs) }}</strong><button @click="emit('openHistory')">查看阅读记录 <span aria-hidden="true">→</span></button></div>
      </article>
      <article class="stat-card">
        <span class="stat-icon" aria-hidden="true">▤</span>
        <div><small>阅读次数</small><strong>{{ readingHistory.totalSessions }}</strong><span class="stat-footnote">{{ readingHistory.books.length }} 本读过的书</span></div>
      </article>
      <article class="stat-card">
        <span class="stat-icon" aria-hidden="true">▣</span>
        <div><small>书架藏书</small><strong>{{ shelf.books.length }}</strong><button @click="emit('openShelf')">整理书架 <span aria-hidden="true">→</span></button></div>
      </article>
      <article class="stat-card">
        <span class="stat-icon" aria-hidden="true">⌕</span>
        <div><small>已启用书源</small><strong>{{ enabledSourceCount }}</strong><button @click="emit('openSources')">管理书源 <span aria-hidden="true">→</span></button></div>
      </article>
    </section>

    <section class="home-discovery" aria-labelledby="home-discovery-title">
      <div class="section-heading discovery-heading">
        <div><p class="home-eyebrow">为你发现</p><h3 id="home-discovery-title">书籍分类</h3></div>
        <div class="home-discovery-actions">
          <button class="home-text-button" @click="emit('openDiscovery')">进入发现 <span aria-hidden="true">→</span></button>
          <button class="home-button home-button-secondary" data-testid="home-config-open" :disabled="busy" @click="emit('manageHome')">管理栏目</button>
        </div>
      </div>
      <div v-if="orderedTabs.length" class="home-tabs" role="tablist" aria-label="首页分类页签">
        <button v-for="tab in orderedTabs" :key="tab.id" :data-testid="`home-tab-${tab.id}`" role="tab" :aria-selected="activeTab?.id === tab.id" :class="{ active: activeTab?.id === tab.id }" @click="activeTabId = tab.id">{{ tab.title }}</button>
      </div>
      <div v-if="!orderedSections.length" data-testid="home-section-empty" class="home-sections-empty">
        <span aria-hidden="true">⌕</span>
        <div><strong>这个页签还没有分类</strong><p>可以在首页设置中添加常用书源分类。</p></div>
        <button class="home-button home-button-secondary" :disabled="busy" @click="emit('openDiscovery')">浏览发现</button>
      </div>
      <div v-else class="home-section-list">
        <article v-for="section in orderedSections" :key="section.id" :data-testid="`home-section-${section.id}`" class="home-section-card" :class="`home-section-style-${section.style}`">
          <header class="home-section-heading">
            <div><p class="home-eyebrow">{{ section.sourceName }} · {{ section.categoryName }}</p><h4>{{ section.title }}</h4></div>
            <button class="home-text-button" :data-testid="`home-section-open-${section.id}`" :disabled="busy || sectionIsLoading(section.id)" @click="emit('openHomeSection', section.id, section.sourceId, section.categoryId)">{{ sectionIsLoading(section.id) ? '加载中…' : '更多' }} <span aria-hidden="true">→</span></button>
          </header>
          <div v-if="sectionIsLoading(section.id) && !sectionResults(section.id).length" class="section-resource-state" aria-live="polite"><span class="mini-loader" aria-hidden="true"></span>正在加载分类内容…</div>
          <div v-else-if="!sectionResults(section.id).length" class="section-resource-state">
            <span>{{ sectionHasPartialError(section.id) ? '部分内容暂时不可用。' : '这个分类还没有加载内容。' }}</span>
            <button class="home-text-button" :disabled="busy || sectionIsLoading(section.id)" @click="emit('openHomeSection', section.id, section.sourceId, section.categoryId)">加载分类 <span aria-hidden="true">→</span></button>
          </div>
          <template v-else>
            <p v-if="sectionHasPartialError(section.id)" class="section-partial-error" role="status">部分来源未能加载，以下为当前可用内容。</p>
            <ol v-if="section.style === 1" class="section-rank-list">
              <li v-for="(result, index) in sectionResults(section.id).slice(0, 5)" :key="result.resultId">
                <span class="rank-number">{{ String(index + 1).padStart(2, "0") }}</span>
                <button class="rank-result" :data-testid="`home-section-result-${result.resultId}`" @click="emit('openResult', result.resultId)"><img v-if="result.coverSrc" :src="result.coverSrc" alt="" loading="lazy" /><span v-else class="section-cover-fallback">{{ result.title.slice(0, 1) }}</span><span class="result-copy"><strong>{{ result.title }}</strong><small>{{ result.author || result.latestChapter || '打开书籍详情' }}</small></span></button>
              </li>
            </ol>
            <div v-else class="section-cover-row" :class="{ 'section-cover-video': section.coverVideo, 'section-four-row': section.style === 3, 'section-infinite-grid': section.style === 2 }">
              <button v-for="result in sectionResults(section.id).slice(0, section.style === 2 ? 8 : 6)" :key="result.resultId" class="section-book-card" :data-testid="`home-section-result-${result.resultId}`" @click="emit('openResult', result.resultId)">
                <img v-if="result.coverSrc" :src="result.coverSrc" alt="" loading="lazy" /><span v-else class="section-cover-fallback">{{ result.title.slice(0, 1) }}</span>
                <span class="section-book-copy"><strong>{{ result.title }}</strong><small>{{ result.author || result.latestChapter || '打开书籍详情' }}</small></span>
              </button>
            </div>
          </template>
        </article>
      </div>
    </section>

    <section class="task-summary" aria-labelledby="home-task-title">
      <div class="section-heading task-heading">
        <div><p class="home-eyebrow">后台处理</p><h3 id="home-task-title">下载与搜索任务</h3></div>
        <button class="home-text-button" @click="emit('openTasks')">查看全部 <span aria-hidden="true">→</span></button>
      </div>
      <div v-if="activeTasks.length" class="task-summary-list">
        <article v-for="task in activeTasks.slice(0, 3)" :key="task.id" class="task-summary-row">
          <div class="task-summary-copy"><strong>{{ taskLabel(task) }}</strong><span>{{ taskStatus(task.status) }}<template v-if="task.total"> · {{ task.completed }} / {{ task.total }}</template></span></div>
          <div class="task-progress" aria-hidden="true"><span :class="{ indeterminate: task.status === 'running' && task.total <= 0 }" :style="{ width: `${taskProgress(task)}%` }"></span></div>
        </article>
        <button v-if="activeTasks.length > 3" class="more-tasks" @click="emit('openTasks')">还有 {{ activeTasks.length - 3 }} 个任务 <span aria-hidden="true">→</span></button>
      </div>
      <div v-else class="task-summary-empty"><span aria-hidden="true">✓</span><p>目前没有正在运行的任务。</p></div>
    </section>
  </section>
</template>

<style scoped>
.home-page { display:grid; gap:22px; color:#35413b; }
.home-intro { display:flex; align-items:flex-end; justify-content:space-between; gap:18px; padding:4px 0 2px; }
.home-eyebrow { margin:0 0 5px; color:#82958a; font-size:12px; font-weight:800; letter-spacing:.13em; text-transform:uppercase; }
.home-intro h2 { margin:0; color:#34433a; font-size:24px; line-height:1.25; }
.home-subtitle { margin:7px 0 0; color:#7b8980; font-size:14px; }
.home-actions { display:flex; flex:none; gap:9px; }
.home-button { min-height:44px; padding:0 15px; border:1px solid transparent; border-radius:8px; font:inherit; font-size:14px; font-weight:700; cursor:pointer; transition:background .15s,border-color .15s,transform .15s; }
.home-button:hover:not(:disabled) { transform:translateY(-1px); }
.home-button:disabled { cursor:wait; opacity:.6; }
.home-button-primary { color:#fff; background:#47745b; }
.home-button-primary:hover:not(:disabled) { background:#3b664e; }
.home-button-secondary { color:#46634f; border-color:#dce7dd; background:#f4f8f3; }
.home-button-secondary:hover:not(:disabled) { background:#eaf2ea; }
.continue-section,.task-summary { min-width:0; padding:19px 21px; border:1px solid #e6ebe6; border-radius:13px; background:#fff; }
.section-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:12px; }
.section-heading h3 { margin:0; color:#3c4c41; font-size:16px; }
.home-text-button,.stat-card button,.more-tasks { min-height:40px; padding:0 4px; border:0; color:#557a61; background:transparent; font:inherit; font-size:13px; cursor:pointer; }
.home-text-button:hover,.stat-card button:hover,.more-tasks:hover { color:#315d43; text-decoration:underline; text-underline-offset:3px; }
.recent-books { display:grid; gap:0; }
.recent-book { display:grid; grid-template-columns:58px minmax(0,1fr) auto; align-items:center; gap:13px; min-height:83px; padding:10px 0; border-top:1px solid #edf0ed; }
.recent-cover { width:58px; height:67px; overflow:hidden; padding:0; border:0; border-radius:6px; background:#eaf0e8; cursor:pointer; }
.recent-cover img { display:block; width:100%; height:100%; object-fit:cover; }
.cover-placeholder { display:grid; width:100%; height:100%; place-items:center; color:#587460; background:linear-gradient(145deg,#edf4e9,#d9e7d9); font-size:23px; font-weight:800; }
.recent-book-info { min-width:0; display:flex; flex-direction:column; gap:4px; }
.recent-title { overflow:hidden; color:#3e4e43; font-size:14px; text-overflow:ellipsis; white-space:nowrap; }
.recent-author,.recent-book-info small { overflow:hidden; color:#7f8c83; font-size:12px; text-overflow:ellipsis; white-space:nowrap; }
.progress-track { height:4px; overflow:hidden; margin-top:3px; border-radius:4px; background:#edf1ed; }
.progress-track span { display:block; height:100%; border-radius:inherit; background:#73a47c; }
.continue-button { min-width:92px; }
.home-empty { display:flex; align-items:center; gap:13px; min-height:94px; padding:14px 0 2px; border-top:1px solid #edf0ed; }
.empty-symbol { display:grid; width:43px; height:43px; flex:none; place-items:center; border-radius:50%; color:#6d8c71; background:#eef4eb; font-size:23px; }
.home-empty > div { min-width:0; flex:1; }
.home-empty strong { color:#495a4d; font-size:14px; }
.home-empty p { margin:4px 0 0; color:#87938b; font-size:13px; }
.home-stat-grid { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:11px; }
.stat-card { min-width:0; display:flex; align-items:flex-start; gap:11px; padding:16px; border:1px solid #e6ebe6; border-radius:11px; background:#fff; }
.stat-icon { display:grid; width:35px; height:35px; flex:none; place-items:center; border-radius:10px; color:#56795f; background:#eef4ed; font-size:17px; }
.stat-card > div { min-width:0; display:flex; flex-direction:column; align-items:flex-start; gap:4px; }
.stat-card small { color:#849087; font-size:12px; }
.stat-card strong { overflow:hidden; max-width:100%; color:#3d5945; font-size:18px; text-overflow:ellipsis; white-space:nowrap; }
.stat-card button,.stat-footnote { min-height:26px; color:#718077; font-size:12px; text-align:left; }
.stat-card button { min-height:26px; padding:0; }
.home-discovery { min-width:0; padding:19px 21px; border:1px solid #e6ebe6; border-radius:13px; background:#fff; }
.discovery-heading { margin-bottom:9px; }
.home-discovery-actions { display:flex; align-items:center; gap:8px; }
.home-tabs { display:flex; gap:5px; overflow-x:auto; margin:0 -2px 13px; padding:0 2px 4px; border-bottom:1px solid #edf0ed; scrollbar-width:thin; }
.home-tabs button { min-height:42px; flex:none; padding:0 13px; border:0; border-bottom:2px solid transparent; color:#77847a; background:transparent; font:inherit; font-size:14px; cursor:pointer; }
.home-tabs button:hover { color:#446950; background:#f7faf6; }
.home-tabs button.active { border-bottom-color:#5e8c68; color:#3f694b; font-weight:750; }
.home-sections-empty { display:flex; align-items:center; gap:12px; min-height:82px; color:#718077; }
.home-sections-empty > span { display:grid; width:38px; height:38px; flex:none; place-items:center; border-radius:50%; color:#6c8b70; background:#eef4eb; font-size:19px; }
.home-sections-empty > div { min-width:0; flex:1; }
.home-sections-empty strong { color:#536258; font-size:14px; }
.home-sections-empty p { margin:4px 0 0; color:#87938b; font-size:12px; }
.home-section-list { display:grid; gap:14px; }
.home-section-card { min-width:0; overflow:hidden; border:1px solid #edf0ed; border-radius:10px; background:#fdfefd; }
.home-section-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; padding:13px 14px 8px; }
.home-section-heading > div { min-width:0; }
.home-section-heading .home-eyebrow { overflow:hidden; margin-bottom:3px; font-size:10px; text-overflow:ellipsis; white-space:nowrap; }
.home-section-heading h4 { overflow:hidden; margin:0; color:#435348; font-size:15px; text-overflow:ellipsis; white-space:nowrap; }
.home-section-heading .home-text-button { flex:none; }
.section-resource-state { display:flex; align-items:center; justify-content:center; gap:8px; min-height:73px; padding:12px; color:#849087; font-size:13px; text-align:center; }
.section-resource-state .home-text-button { margin-left:5px; }
.mini-loader { width:15px; height:15px; border:2px solid #d9e6db; border-top-color:#5c8965; border-radius:50%; animation:home-spin .8s linear infinite; }
.section-partial-error { margin:0; padding:6px 14px; color:#88724f; background:#faf6ec; font-size:12px; }
.section-cover-row { display:flex; gap:10px; overflow-x:auto; padding:8px 14px 14px; scrollbar-width:thin; }
.section-book-card { width:112px; min-width:0; flex:none; display:flex; flex-direction:column; overflow:hidden; padding:0; border:1px solid #e8ece8; border-radius:7px; background:#fff; text-align:left; cursor:pointer; }
.section-book-card:hover,.rank-result:hover { border-color:#c5d8c8; background:#f8fbf7; }
.section-book-card > img,.section-book-card > .section-cover-fallback { display:block; width:100%; aspect-ratio:3/4; object-fit:cover; }
.section-cover-row.section-cover-video .section-book-card > img,.section-cover-row.section-cover-video .section-book-card > .section-cover-fallback { aspect-ratio:16/9; }
.section-cover-fallback { display:grid; place-items:center; color:#66806a; background:linear-gradient(145deg,#edf4e9,#d9e7d9); font-size:23px; font-weight:750; }
.section-book-copy { display:flex; min-width:0; flex-direction:column; gap:4px; padding:7px 8px 9px; }
.section-book-copy strong,.section-book-copy small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.section-book-copy strong { color:#46564b; font-size:12px; }
.section-book-copy small { color:#859087; font-size:11px; }
.section-cover-row.section-infinite-grid,.section-cover-row.section-four-row { display:grid; grid-template-columns:repeat(4,minmax(80px,1fr)); overflow:visible; }
.section-cover-row.section-infinite-grid .section-book-card,.section-cover-row.section-four-row .section-book-card { width:auto; min-width:0; }
.section-cover-row.section-infinite-grid .section-book-card > img,.section-cover-row.section-infinite-grid .section-book-card > .section-cover-fallback { aspect-ratio:3/4; }
.section-rank-list { display:grid; gap:0; margin:0; padding:0 14px 8px; list-style:none; }
.section-rank-list li { display:grid; grid-template-columns:27px minmax(0,1fr); align-items:center; gap:7px; min-height:62px; border-top:1px solid #edf0ed; }
.rank-number { color:#94a197; font-size:12px; font-variant-numeric:tabular-nums; }
.rank-result { min-width:0; display:flex; align-items:center; gap:9px; min-height:55px; padding:4px; border:1px solid transparent; border-radius:6px; background:transparent; text-align:left; cursor:pointer; }
.rank-result img,.rank-result .section-cover-fallback { width:35px; height:47px; flex:none; border-radius:3px; object-fit:cover; font-size:16px; }
.result-copy { min-width:0; display:flex; flex-direction:column; gap:4px; }
.result-copy strong,.result-copy small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.result-copy strong { color:#4c5a50; font-size:13px; }
.result-copy small { color:#87938b; font-size:11px; }
.task-heading { margin-bottom:7px; }
.task-summary-list { border-top:1px solid #edf0ed; }
.task-summary-row { display:grid; grid-template-columns:minmax(0,1fr) minmax(80px,20%); align-items:center; gap:16px; min-height:56px; border-bottom:1px solid #f0f2f0; }
.task-summary-copy { min-width:0; display:flex; justify-content:space-between; gap:12px; }
.task-summary-copy strong { overflow:hidden; color:#526157; font-size:13px; font-weight:650; text-overflow:ellipsis; white-space:nowrap; }
.task-summary-copy span { flex:none; color:#87938b; font-size:12px; }
.task-progress { height:4px; overflow:hidden; border-radius:5px; background:#edf1ed; }
.task-progress span { display:block; height:100%; border-radius:inherit; background:#75a27c; }
.task-progress span.indeterminate { width:35%!important; animation:task-slide 1.2s ease-in-out infinite alternate; }
.task-summary-empty { display:flex; align-items:center; gap:9px; min-height:51px; padding-top:5px; border-top:1px solid #edf0ed; color:#7c8980; }
.task-summary-empty > span { color:#6b9870; font-size:17px; }
.task-summary-empty p { margin:0; font-size:13px; }
.more-tasks { display:block; width:100%; text-align:right; }
@keyframes task-slide { from { transform:translateX(-15%); } to { transform:translateX(190%); } }
@keyframes home-spin { to { transform:rotate(360deg); } }
@media(max-width:900px) { .home-stat-grid { grid-template-columns:repeat(2,minmax(0,1fr)); } }
@media(max-width:620px) {
  .home-discovery-actions { align-items:stretch; flex-direction:column; }
  .home-discovery-actions > * { width:100%; }
  .home-page { gap:15px; }
  .home-intro { align-items:flex-start; flex-direction:column; gap:13px; }
  .home-intro h2 { font-size:21px; }
  .home-actions { width:100%; }
  .home-actions .home-button { flex:1; }
  .continue-section,.home-discovery,.task-summary { padding:15px 13px; }
  .recent-book { grid-template-columns:49px minmax(0,1fr); gap:10px; }
  .recent-cover { width:49px; height:61px; }
  .continue-button { grid-column:2; justify-self:start; min-height:38px; }
  .home-empty { flex-wrap:wrap; }
  .home-empty .home-button { margin-left:56px; }
  .home-stat-grid { gap:8px; }
  .stat-card { gap:8px; padding:12px 10px; }
  .stat-icon { width:30px; height:30px; }
  .stat-card strong { font-size:16px; }
  .task-summary-row { grid-template-columns:1fr; gap:6px; padding:9px 0; }
  .task-summary-copy { flex-direction:column; gap:3px; }
  .home-section-heading { padding:11px 10px 7px; }
  .section-cover-row { gap:7px; padding:7px 10px 11px; }
  .section-cover-row.section-infinite-grid,.section-cover-row.section-four-row { grid-template-columns:repeat(2,minmax(0,1fr)); }
  .home-sections-empty { flex-wrap:wrap; }
  .home-sections-empty .home-button { margin-left:50px; }
}
</style>
