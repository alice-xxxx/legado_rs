<script setup lang="ts">
// 页面直接显示全局 history 中的阅读记录；改动由 historyStore 自动保存。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { bookPreviews, loadBookPreviews } from "../books/bookDetailState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { backupBusy } from "./backupState";
import { readingStatistics } from "./readingStatistics";
import type { AppBootstrap, ReadingHistoryResource, ReadingStatisticsResource } from "../../api/types";
import { history as appHistory } from "./historyStore";
import router, { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";

type ReadingStatisticsPeriod = "today" | "7days" | "30days" | "all";

const blocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);

async function openHistoryBook(bookId: string): Promise<void> {
  if (blocked.value) return;
  const requestId = requestBookDetailOpen({ kind: "cached-book", bookId, initialTab: "catalog" });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(error instanceof Error ? error.message : String(error), "error");
  }
}


const history = computed(() => appHistory.value.reading);
const statisticsPeriod = ref<ReadingStatisticsPeriod>("30days");
const periodStatistics = ref<ReadingStatisticsResource>({ books: [], days: [], totalDurationMs: 0, totalSessions: 0 });
const loading = ref(true);
const loadError = ref("");
const shelfError = ref("");
const statisticsBusy = ref(false);
const statisticsError = ref("");
const historyQuery = ref("");
const historyVisibleLimit = ref(50);
const historySearchOpen = ref(false);
const historyClearPending = ref(false);
const historyDeletePendingBookId = ref<string | null>(null);

const statisticsData = computed<ReadingStatisticsResource>(() => statisticsPeriod.value === "all" ? history.value : periodStatistics.value);
const days = computed(() => statisticsData.value.days);
const books = computed(() => statisticsData.value.books);
const totalDurationMs = computed(() => statisticsData.value.totalDurationMs);
const totalSessions = computed(() => statisticsData.value.totalSessions);
const allTimeTotalSessions = computed(() => history.value.totalSessions);
watch(() => history.value.books.map((book) => book.bookId), (ids) => { void loadBookPreviews(ids); }, { immediate: true });

let disposed = false;
let restoreRevision = 0;
let loadRevision = 0;
let statisticsRequestRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function localDateKey(date: Date): string {
  return `${date.getFullYear()}${String(date.getMonth() + 1).padStart(2, "0")}${String(date.getDate()).padStart(2, "0")}`;
}

function readingStatisticsDateRange(period: Exclude<ReadingStatisticsPeriod, "all">, now = new Date()): { fromMs: number; toMs: number } {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const to = new Date(today);
  to.setDate(to.getDate() + 1);
  const from = new Date(today);
  const dayCount = period === "today" ? 1 : period === "7days" ? 7 : 30;
  from.setDate(from.getDate() - dayCount + 1);
  return { fromMs: from.getTime(), toMs: to.getTime() };
}

async function refreshStatistics(): Promise<void> {
  const requestRevision = ++statisticsRequestRevision;
  const currentRestoreRevision = restoreRevision;
  const period = statisticsPeriod.value;
  statisticsError.value = "";
  if (period === "all") {
    statisticsBusy.value = false;
    return;
  }
  statisticsBusy.value = true;
  try {
    const { fromMs, toMs } = readingStatisticsDateRange(period);
    const next = readingStatistics(history.value, fromMs, toMs);
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== statisticsRequestRevision) return;
    periodStatistics.value = next;
  } catch (error) {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === statisticsRequestRevision) {
      statisticsError.value = errorText(error);
    }
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === statisticsRequestRevision) {
      statisticsBusy.value = false;
    }
  }
}

watch(() => history.value.sessions, () => { void refreshStatistics(); }, { deep: true });

function changeStatisticsPeriod(period: ReadingStatisticsPeriod): void {
  if (blocked.value) return;
  statisticsPeriod.value = period;
  historyVisibleLimit.value = 50;
  void refreshStatistics();
}

async function loadPageData(_snapshot?: AppBootstrap): Promise<void> {
  const requestRevision = ++loadRevision;
  const currentRestoreRevision = restoreRevision;
  loading.value = true;
  loadError.value = "";
  try {
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== loadRevision) return;
    await refreshStatistics();
  } catch (error) {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === loadRevision) {
      loadError.value = errorText(error);
    }
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === loadRevision) loading.value = false;
  }
}

function handleRestore(snapshot: AppBootstrap): void {
  restoreRevision += 1;
  loadRevision += 1;
  statisticsRequestRevision += 1;
  historyClearPending.value = false;
  historyDeletePendingBookId.value = null;
  void loadPageData(snapshot);
}

function mutateHistory(action: { kind: "clear" } | { kind: "deleteBook"; bookId: string }): void {
  if (blocked.value || loading.value) return;
  statisticsRequestRevision += 1;
  statisticsBusy.value = false;
  if (action.kind === "clear") {
    appHistory.value.reading = { sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 };
  } else {
    appHistory.value.reading.sessions = appHistory.value.reading.sessions
      .filter((session) => session.bookId !== action.bookId);
  }
  historyClearPending.value = false;
  historyDeletePendingBookId.value = null;
}

function confirmHistoryBookDeletion(): void {
  const bookId = historyDeletePendingBookId.value;
  if (bookId) void mutateHistory({ kind: "deleteBook", bookId });
}

function confirmHistoryClear(): void {
  void mutateHistory({ kind: "clear" });
}

function returnToSettings(): void {
  returnToRoute(routeNames.settings);
}

const canOpenHistoryBook = (bookId: string) => Boolean(bookId);
function coverForBook(bookId: string): string | undefined { return bookPreviews.value[bookId]?.coverSrc; }
function title(bookId: string): string { return bookPreviews.value[bookId]?.title ?? "缓存书信息暂不可用"; }
function historyBookSubtitle(book: ReadingHistoryResource["books"][number]): string {
  const shelfBook = bookPreviews.value[book.bookId];
  const chapterIndex = shelfBook?.progress?.chapterIndex;
  if (shelfBook && typeof chapterIndex === "number" && Number.isInteger(chapterIndex)
    && chapterIndex >= 0 && shelfBook.chapterCount > 0) {
    const chapter = Math.min(chapterIndex + 1, shelfBook.chapterCount);
    return `第 ${chapter} 章 · ${Math.round(chapter / shelfBook.chapterCount * 100)}%`;
  }
  return `${book.sessionCount} 次阅读 · ${formatDuration(book.totalDurationMs)}`;
}
function formatHistoryTotal(value: number): string {
  if (value <= 0) return "0m";
  const minutes = Math.floor(value / 60_000);
  if (!minutes) return "<1m";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const remaining = minutes % 60;
  return remaining ? `${hours}h ${remaining}m` : `${hours}h`;
}
const filteredHistoryBooks = computed(() => {
  const query = historyQuery.value.trim().toLocaleLowerCase();
  return books.value.filter((book) => !query || title(book.bookId).toLocaleLowerCase().includes(query))
    .sort((left, right) => right.lastReadAtMs - left.lastReadAtMs);
});
const visibleHistoryBooks = computed(() => filteredHistoryBooks.value.slice(0, historyVisibleLimit.value));
watch(historyQuery, () => { historyVisibleLimit.value = 50; });
const dayMap = computed(() => new Map(days.value.map((day) => [day.day, day])));
const heatmap = computed(() => {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const cells: Array<{ key: string; date: Date; duration: number; sessions: number }> = [];
  const dayCount = statisticsPeriod.value === "today" ? 1 : statisticsPeriod.value === "7days" ? 7 : statisticsPeriod.value === "30days" ? 30 : 91;
  for (let offset = dayCount - 1; offset >= 0; offset -= 1) {
    const date = new Date(today);
    date.setDate(date.getDate() - offset);
    const entry = dayMap.value.get(localDateKey(date));
    cells.push({ key: localDateKey(date), date, duration: entry?.totalDurationMs ?? 0, sessions: entry?.sessionCount ?? 0 });
  }
  return cells;
});

function heatLevel(duration: number): number {
  if (!duration) return 0;
  const minutes = duration / 60_000;
  if (minutes < 15) return 1;
  if (minutes < 45) return 2;
  if (minutes < 90) return 3;
  return 4;
}

function formatDuration(value: number): string {
  const minutes = Math.floor(value / 60_000);
  if (minutes < 1) return value > 0 ? "不足 1 分钟" : "0 分钟";
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return hours ? `${hours} 小时 ${rest} 分钟` : `${minutes} 分钟`;
}

/** Render actual session timestamps relative to the user's device-local calendar. */
function historyTimeLabel(timestamp: number): string {
  const value = new Date(timestamp);
  const today = new Date();
  const dayStart = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const eventDay = new Date(value.getFullYear(), value.getMonth(), value.getDate());
  const daysAgo = Math.round((Date.UTC(dayStart.getFullYear(), dayStart.getMonth(), dayStart.getDate())
    - Date.UTC(eventDay.getFullYear(), eventDay.getMonth(), eventDay.getDate())) / 86_400_000);
  const time = value.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
  if (daysAgo === 0) return `今天 ${time}`;
  if (daysAgo === 1) return `昨天 ${time}`;
  if (daysAgo >= 2 && daysAgo <= 6) return `${value.toLocaleDateString("zh-CN", { weekday: "long" })} ${time}`;
  return `${value.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" })} ${time}`;
}

function toggleHistorySearch(): void {
  historySearchOpen.value = !historySearchOpen.value;
  if (!historySearchOpen.value) historyQuery.value = "";
}

function dayLabel(date: Date): string {
  return date.toLocaleDateString("zh-CN", { month: "long", day: "numeric" });
}

async function setupEvents(): Promise<void> {
  try {
    const unlistenHistory = await listen("history-updated", () => { void refreshStatistics(); });
    if (disposed) { unlistenHistory(); return; }
    unlisteners.push(unlistenHistory);
    const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => handleRestore(event.payload));
    if (disposed) { unlistenRestore(); return; }
    unlisteners.push(unlistenRestore);
  } catch (error) {
    console.debug("Native reading history listeners unavailable:", errorText(error));
  }
}

onMounted(async () => {
  await setupEvents();
  await loadPageData();
});

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  loadRevision += 1;
  statisticsRequestRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
});
</script>

<template>
  <header class="history-topbar safe-area-top">
    <BackButton label="返回我的" @click="returnToSettings" />
    <div class="history-heading">
      <strong>历史</strong>
    </div>
    <span></span>
  </header>
  <section class="my-records my-records--history">
    <p v-if="loading" class="record-state" role="status">正在加载阅读记录…</p>
    <div v-else-if="loadError" class="record-state record-error" role="alert">
      阅读记录载入失败：{{ loadError }} <button type="button" @click="loadPageData()">重试</button>
    </div>
    <template v-else>
      <p v-if="shelfError" class="record-state record-error" role="alert">书架更新失败：{{ shelfError }} <button type="button" @click="loadPageData()">重试</button></p>
      <div class="records-toolbar">
        <nav class="record-periods">
          <button v-for="option in ([['today','今天'],['7days','7 天'],['30days','30 天'],['all','全部']] as const)" :key="option[0]"
            type="button" :class="{ active: statisticsPeriod === option[0] }"
            :disabled="blocked || statisticsBusy" @click="changeStatisticsPeriod(option[0])">{{ option[1] }}</button>
        </nav>
        <div class="record-toolbar-actions">
          <button v-if="allTimeTotalSessions" type="button" class="record-icon-button" aria-label="搜索阅读历史" @click="toggleHistorySearch"><PrototypeIcon name="search"/></button>
          <details v-if="allTimeTotalSessions" class="record-row-menu record-clear-menu">
            <summary><PrototypeIcon name="more"/></summary>
            <div><button type="button" class="record-delete" :disabled="blocked" @click="historyClearPending = true">清空全部历史记录</button></div>
          </details>
        </div>
      </div>
      <div v-if="statisticsBusy" class="record-state">正在读取阅读记录…</div>
      <div v-else-if="statisticsError" class="record-state record-error">
        {{ statisticsError }} <button type="button" @click="refreshStatistics">重试</button>
      </div>
      <template v-else>
        <div class="record-stats">
          <div><strong>{{ formatHistoryTotal(totalDurationMs) }}</strong><small>累计时长</small></div>
          <div><strong>{{ totalSessions }}</strong><small>打开次数</small></div>
          <div><strong>{{ books.length }}</strong><small>内容数</small></div>
        </div>
        <label v-if="books.length && historySearchOpen" class="record-search"><PrototypeIcon name="search"/>
          <input v-model="historyQuery" type="search" placeholder="查找读过的内容"/>
        </label>
        <div v-if="visibleHistoryBooks.length" class="record-timeline">
          <article v-for="book in visibleHistoryBooks" :key="book.bookId" class="record-timeline-row"
            >
            <time :datetime="new Date(book.lastReadAtMs).toISOString()"
              :title="new Date(book.lastReadAtMs).toLocaleString('zh-CN')">{{ historyTimeLabel(book.lastReadAtMs) }}</time>
            <button type="button" class="record-book-icon record-book-open"
              :aria-label="`继续阅读《${title(book.bookId)}》`"
              :disabled="!canOpenHistoryBook(book.bookId) || blocked"
              @click="openHistoryBook(book.bookId)">
              <img v-if="coverForBook(book.bookId)" :src="coverForBook(book.bookId)"
                loading="lazy" decoding="async"/>
              <span v-else class="record-cover-placeholder"></span>
            </button>
            <div class="record-book-copy">
              <button type="button" class="record-book-title"
                :disabled="!canOpenHistoryBook(book.bookId) || blocked"
                @click="openHistoryBook(book.bookId)">{{ title(book.bookId) }}</button>
              <small>{{ historyBookSubtitle(book) }}</small>
              <small v-if="!canOpenHistoryBook(book.bookId)">书籍已不在内容库，可删除这条历史记录</small>
            </div>
            <details class="record-row-menu">
              <summary><PrototypeIcon name="more"/></summary>
              <div>
                <button v-if="canOpenHistoryBook(book.bookId)" type="button"
                  :disabled="blocked"
                  @click="openHistoryBook(book.bookId)">打开内容</button>
                <button type="button" class="record-delete"
                  :disabled="blocked"
                  @click="historyDeletePendingBookId = book.bookId">删除这本书的历史</button>
              </div>
            </details>
          </article>
        </div>
        <div v-else class="record-state">{{ historyQuery.trim() ? '没有匹配的阅读记录。' : '当前时间范围内没有阅读记录。' }}</div>
        <button v-if="filteredHistoryBooks.length > visibleHistoryBooks.length" type="button" class="record-load-more"
          @click="historyVisibleLimit += 50">查看更多历史 · {{ visibleHistoryBooks.length }} / {{ filteredHistoryBooks.length }}</button>
        <details v-if="days.length" class="record-activity">
          <summary><PrototypeIcon name="history"/> 阅读活跃度</summary>
          <div class="record-heatmap">
            <span v-for="cell in heatmap" :key="cell.key" :class="'heat-' + heatLevel(cell.duration)"
              :title="dayLabel(cell.date) + ' · ' + cell.sessions + ' 次 · ' + formatDuration(cell.duration)"></span>
          </div>
          <p>颜色越深，表示当天累计阅读时间越长。统计按真实记录生成。</p>
        </details>
      </template>
      <div v-if="historyDeletePendingBookId" class="record-confirm-backdrop" data-back-layer>
        <div class="record-confirm">
          <strong>删除《{{ title(historyDeletePendingBookId) }}》的阅读记录？</strong>
          <p>这本书的阅读会话与时长统计将被删除，操作不可撤销。</p>
          <footer>
            <button type="button" class="record-action" data-back-action @click="historyDeletePendingBookId = null">取消</button>
            <button type="button" class="record-action record-delete"
              :disabled="blocked" @click="confirmHistoryBookDeletion">确认删除</button>
          </footer>
        </div>
      </div>
      <div v-if="historyClearPending" class="record-confirm-backdrop" data-back-layer>
        <div class="record-confirm"><strong>清空全部阅读历史？</strong>
          <p>将删除已记录的阅读历史和统计，无法撤销。</p>
          <footer><button type="button" class="record-action" data-back-action @click="historyClearPending = false">取消</button>
            <button type="button" class="record-action record-delete" :disabled="blocked" @click="confirmHistoryClear">确认清空</button></footer>
        </div>
      </div>
    </template>
  </section>
</template>
<style scoped>
.my-records{width:min(940px,100%);margin:0 auto;padding:8px 0 50px;color:var(--app-text)}
.my-records--history{width:min(860px,calc(100% - 36px));padding:24px 0 54px}
.history-topbar{position:sticky;top:0;z-index:35;min-height:var(--ui-header-height);display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;padding-inline:var(--ui-header-gutter);border-bottom:1px solid var(--app-line);background:var(--app-header);backdrop-filter:blur(18px)}
.history-heading{display:flex;flex-direction:column;gap:2px;min-width:0}
.history-heading strong{font-size:var(--ui-font-heading);font-weight:750;color:var(--app-text);line-height:1.25}
.records-toolbar{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:10px}
.record-periods{display:flex;align-items:center;gap:4px}
.record-periods button{height:30px;min-height:30px;padding:0 10px;border:0;border-radius:9px;background:transparent;color:var(--app-muted);font-size:var(--ui-font-body);cursor:pointer}
.record-periods button.active{color:var(--app-text);font-weight:750}
.record-action{padding:7px 9px;border:0;border-radius:8px;background:transparent;color:var(--app-accent);font-size:var(--ui-font-body);cursor:pointer}
.record-action:disabled{opacity:.5;cursor:default}
.record-delete{color:var(--app-danger)}
.record-stats{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;margin:18px 0 20px}
.record-stats>div{display:grid;gap:5px;padding:13px;border:1px solid var(--app-line);border-radius:12px;background:var(--app-surface)}
.record-stats strong{font-size:var(--ui-font-heading);color:var(--app-text);overflow-wrap:anywhere}
.record-stats small{font-size:var(--ui-font-caption);color:var(--app-subtle)}
.record-search{display:flex;align-items:center;gap:9px;box-sizing:border-box;width:min(420px,100%);height:40px;padding:0 11px;border:1px solid var(--app-line);border-radius:10px;background:var(--app-surface)}
.record-search>.prototype-icon{width:18px;height:18px;color:var(--app-subtle)}
.record-search input{flex:1;min-width:0;border:0;outline:0;color:var(--app-text);background:transparent;font-size:var(--ui-font-body)}
.record-timeline{margin-top:12px;border-top:1px solid var(--app-line)}
.record-timeline-row{display:grid;grid-template-columns:88px 45px minmax(0,1fr) 36px;gap:10px;align-items:center;min-height:68px;padding:7px 0;border-bottom:1px solid var(--app-line)}
.record-timeline-row time{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.5}
.record-book-icon{display:grid;place-items:center;flex:none;width:45px;height:54px;border-radius:7px;background:#546c72;color:#fff;overflow:hidden}
.record-book-icon>.prototype-icon{width:21px;height:21px}
.record-book-icon img{display:block;width:100%;height:100%;border-radius:inherit;object-fit:cover}
.record-book-copy{display:flex;flex-direction:column;gap:3px;min-width:0}
.record-book-title{width:100%;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding:0;border:0;background:none;color:var(--app-text);text-align:left;font-size:var(--ui-font-caption);font-weight:750;cursor:pointer}
.record-book-title:disabled{opacity:1;color:var(--app-muted);cursor:default}
.record-book-open{padding:0;border:0;cursor:pointer}
.record-book-open:disabled{opacity:.65;cursor:default}
.record-cover-placeholder{position:relative;display:block;width:100%;height:100%;background:linear-gradient(140deg,#506f73,#253c42)}
.record-cover-placeholder::after{content:"";position:absolute;top:15%;right:-25%;height:70%;width:75%;border-radius:50%;background:#ffffff25}
.record-book-copy small{font-size:var(--ui-font-caption);color:var(--app-subtle);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.record-state{display:flex;align-items:center;justify-content:center;min-height:105px;padding:20px 0;color:var(--app-subtle);font-size:var(--ui-font-caption);text-align:center}
.record-error{color:var(--app-danger)}
.record-error button{border:0;background:none;color:var(--app-accent);cursor:pointer}
.record-activity{margin:23px 0 0;padding:14px 0;border-top:1px solid var(--app-line)}
.record-activity summary{display:flex;align-items:center;gap:8px;cursor:pointer;color:var(--app-muted);font-size:var(--ui-font-caption)}
.record-activity summary>.prototype-icon{width:17px;height:17px;color:var(--app-accent)}
.record-heatmap{display:grid;grid-auto-flow:column;grid-template-rows:repeat(7,11px);grid-auto-columns:11px;gap:4px;overflow-x:auto;margin:14px 0}
.record-heatmap span{border-radius:3px;background:var(--app-panel-soft)}
.record-heatmap .heat-1{background:var(--app-accent-soft)}.record-heatmap .heat-2{background:#b6a9eb}.record-heatmap .heat-3{background:#8371d7}.record-heatmap .heat-4{background:var(--app-accent)}
.record-activity p{color:var(--app-subtle);font-size:var(--ui-font-caption)}
.record-load-more{display:block;margin:18px auto 9px;padding:10px 16px;border:1px solid var(--app-accent-soft);border-radius:10px;background:var(--app-accent-soft);color:var(--app-accent);font-size:var(--ui-font-caption);cursor:pointer}
.record-confirm-backdrop{position:fixed;inset:0;z-index:80;display:grid;place-items:center;padding:14px;background:#1d1d2b66}
.record-confirm{box-sizing:border-box;width:min(420px,100%);padding:22px;border-radius:15px;background:var(--app-surface);box-shadow:0 15px 45px #11122224}
.record-confirm strong{font-size:var(--ui-font-body)}
.record-confirm p{font-size:var(--ui-font-caption);color:var(--app-muted)}
.record-confirm footer{display:flex;justify-content:flex-end;gap:12px;margin-top:17px}
.record-toolbar-actions{display:flex;align-items:center;gap:6px;margin-left:auto}
.record-icon-button{display:grid;place-items:center;width:35px;height:35px;border:0;border-radius:9px;background:transparent;color:var(--app-muted);cursor:pointer}
.record-icon-button svg{width:19px;height:19px}
.record-row-menu{position:relative}
.record-row-menu summary{display:grid;place-items:center;width:36px;height:36px;border-radius:9px;color:var(--app-muted);cursor:pointer;list-style:none}
.record-row-menu summary::-webkit-details-marker{display:none}
.record-row-menu summary svg{width:19px;height:19px}
.record-row-menu>div{position:absolute;right:0;top:39px;z-index:9;min-width:158px;padding:6px;border:1px solid var(--app-line);border-radius:10px;background:var(--app-surface);box-shadow:0 14px 32px #25223520}
.record-row-menu>div button{display:block;width:100%;min-height:34px;padding:6px 9px;border:0;border-radius:6px;background:transparent;text-align:left;font-size:var(--ui-font-body);cursor:pointer}
@media(max-width:660px){.history-topbar{padding-inline:var(--ui-header-gutter)}.my-records--history{width:calc(100% - 36px);padding-top:24px}.record-timeline-row{grid-template-columns:76px 45px minmax(0,1fr) 36px;gap:8px}.record-timeline-row time{font-size:var(--ui-font-caption)}.record-book-icon{width:45px;height:54px}.record-book-title{font-size:var(--ui-font-caption)}}
@media(max-width:400px){.record-timeline-row{grid-template-columns:64px 40px minmax(0,1fr) 32px;gap:6px}.record-book-icon{width:40px;height:49px}.record-stats>div{padding:11px 9px}}
</style>
