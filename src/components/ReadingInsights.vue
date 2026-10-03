<script setup lang="ts">
import { computed } from "vue";

export interface ReadingDay {
  day: string;
  sessionCount: number;
  totalDurationMs: number;
}

export interface ReadingBookSummary {
  bookId: string;
  sessionCount: number;
  totalDurationMs: number;
  lastReadAtMs: number;
}

export interface BookmarkEntry {
  id: string;
  bookId: string;
  chapterIndex: number;
  chapterTitle?: string;
  offset: number;
  note: string;
  orphaned?: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

const props = defineProps<{
  days: ReadingDay[];
  books: ReadingBookSummary[];
  bookmarks: BookmarkEntry[];
  bookTitles: Record<string, string>;
  totalDurationMs: number;
  totalSessions: number;
}>();

const emit = defineEmits<{
  clearHistory: [];
  deleteBookHistory: [bookId: string];
  openBookmark: [bookmark: BookmarkEntry];
  deleteBookmark: [bookmarkId: string];
}>();

function localDateKey(date: Date): string {
  return `${date.getFullYear()}${String(date.getMonth() + 1).padStart(2, "0")}${String(date.getDate()).padStart(2, "0")}`;
}

const dayMap = computed(() => new Map(props.days.map((day) => [day.day, day])));
const heatmap = computed(() => {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const cells: Array<{ key: string; date: Date; duration: number; sessions: number }> = [];
  for (let offset = 90; offset >= 0; offset -= 1) {
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
  if (minutes < 1) return "不足 1 分钟";
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return hours ? `${hours} 小时 ${rest} 分钟` : `${minutes} 分钟`;
}

function title(bookId: string): string {
  return props.bookTitles[bookId] ?? "书架中已移除的书";
}

function dayLabel(date: Date): string {
  return date.toLocaleDateString("zh-CN", { month: "long", day: "numeric" });
}
</script>

<template>
  <section class="insights-grid">
    <article class="insight-card history-card">
      <header class="insight-heading">
        <div><p class="eyebrow">阅读记录</p><h3>每天读一点，故事就会继续。</h3></div>
        <button v-if="totalSessions" class="quiet-action" @click="emit('clearHistory')">清空记录</button>
      </header>
      <div class="reading-totals">
        <div><strong>{{ formatDuration(totalDurationMs) }}</strong><span>累计阅读</span></div>
        <div><strong>{{ totalSessions }}</strong><span>阅读次数</span></div>
        <div><strong>{{ books.length }}</strong><span>读过的书</span></div>
      </div>
      <div class="heatmap-wrap">
        <div class="heatmap-months"><span>近 13 周</span><span>阅读越多，颜色越深</span></div>
        <div class="reading-heatmap" role="img" aria-label="近 13 周阅读热度">
          <span v-for="cell in heatmap" :key="cell.key" class="heat-cell" :class="`level-${heatLevel(cell.duration)}`"
            :title="`${dayLabel(cell.date)} · ${cell.sessions} 次 · ${formatDuration(cell.duration)}`"></span>
        </div>
        <div class="heatmap-legend"><span>少</span><i class="level-0"></i><i class="level-1"></i><i class="level-2"></i><i class="level-3"></i><i class="level-4"></i><span>多</span></div>
      </div>
      <div v-if="books.length" class="insight-book-list">
        <div class="list-heading"><strong>最近读过</strong><span>累计时长</span></div>
        <article v-for="book in books.slice(0, 5)" :key="book.bookId" class="insight-book-row">
          <div><strong>{{ title(book.bookId) }}</strong><small>{{ book.sessionCount }} 次 · 最近 {{ new Date(book.lastReadAtMs).toLocaleDateString() }}</small></div>
          <span>{{ formatDuration(book.totalDurationMs) }}</span>
          <button class="quiet-action" :aria-label="`删除${title(book.bookId)}的阅读记录`" @click="emit('deleteBookHistory', book.bookId)">删除</button>
        </article>
      </div>
      <div v-else class="insight-empty">合上一本书后，这里会记录你的阅读时光。</div>
    </article>

    <article class="insight-card bookmarks-card">
      <header class="insight-heading"><div><p class="eyebrow">书签</p><h3>留住想再读的那一页。</h3></div><span class="count-pill">{{ bookmarks.length }}</span></header>
      <div v-if="bookmarks.length" class="bookmark-list">
        <article v-for="bookmark in [...bookmarks].sort((a, b) => b.updatedAtMs - a.updatedAtMs)" :key="bookmark.id" class="bookmark-row">
          <button class="bookmark-open" :data-testid="`bookmark-open-${bookmark.id}`" :disabled="bookmark.orphaned" :aria-label="bookmark.orphaned ? `旧章节书签：${bookmark.chapterTitle || `第 ${bookmark.chapterIndex + 1} 章`}，无法打开` : `打开${title(bookmark.bookId)}书签`" @click="emit('openBookmark', bookmark)">
            <span class="bookmark-mark">▮</span><span class="bookmark-copy"><strong>{{ title(bookmark.bookId) }}</strong><small v-if="bookmark.orphaned" class="bookmark-orphaned-label">旧章节：{{ bookmark.chapterTitle || `第 ${bookmark.chapterIndex + 1} 章` }} · 无法匹配</small><small v-else>第 {{ bookmark.chapterIndex + 1 }} 章 · 第 {{ bookmark.offset + 1 }} 页</small><em v-if="bookmark.note">{{ bookmark.note }}</em></span>
          </button>
          <button class="quiet-action" :aria-label="`删除${title(bookmark.bookId)}书签`" @click="emit('deleteBookmark', bookmark.id)">移除</button>
        </article>
      </div>
      <div v-else class="insight-empty">阅读时点按书签，就能从这里回到那一页。</div>
    </article>
  </section>
</template>

<style scoped>
.insights-grid { display:grid; grid-template-columns:minmax(0,1.25fr) minmax(290px,.75fr); gap:16px; align-items:start; }
.insight-card { min-width:0; padding:22px; border:1px solid #e7ebe7; border-radius:14px; background:#fff; }
.insight-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; }
.insight-heading .eyebrow { margin:0 0 4px; color:#91a294; font-size:13px; font-weight:800; letter-spacing:.14em; }
.insight-heading h3 { margin:0; color:#35413b; font-size:15px; line-height:1.5; }
.quiet-action { flex:none; min-height:44px; padding:0 9px; border:0; border-radius:6px; color:#53665a; background:transparent; font-size:14px; cursor:pointer; }
.quiet-action:hover { color:#3c725f; background:#edf4ef; }
.reading-totals { display:grid; grid-template-columns:repeat(3,1fr); gap:8px; margin:20px 0 17px; }
.reading-totals div { display:flex; flex-direction:column; gap:5px; padding:12px; border-radius:10px; background:#f6f8f5; }
.reading-totals strong { color:#446c55; font-size:15px; }
.reading-totals span,.heatmap-months,.heatmap-legend { color:#9aa39d; font-size:13px; }
.heatmap-wrap { padding:14px 13px 10px; border:1px solid #edf0ed; border-radius:10px; }
.heatmap-months { display:flex; justify-content:space-between; margin-bottom:10px; }
.reading-heatmap { display:grid; grid-auto-flow:column; grid-template-rows:repeat(7,10px); grid-auto-columns:10px; gap:4px; overflow-x:auto; padding-bottom:4px; }
.heat-cell,.heatmap-legend i { width:10px; height:10px; border-radius:3px; background:#eef2ee; }
.heat-cell.level-1,.heatmap-legend .level-1 { background:#d5e8d9; }.heat-cell.level-2,.heatmap-legend .level-2 { background:#a8cfae; }.heat-cell.level-3,.heatmap-legend .level-3 { background:#71aa7b; }.heat-cell.level-4,.heatmap-legend .level-4 { background:#3c725f; }
.heatmap-legend { display:flex; align-items:center; justify-content:flex-end; gap:4px; margin-top:8px; }
.heatmap-legend i { display:inline-block; }
.insight-book-list { margin-top:18px; }
.list-heading,.insight-book-row { display:grid; grid-template-columns:minmax(0,1fr) auto 42px; align-items:center; gap:9px; }
.list-heading { padding:0 4px 7px; border-bottom:1px solid #edf0ed; color:#a0a9a3; font-size:13px; }
.list-heading strong { color:#68756c; font-size:14px; }
.insight-book-row { min-height:51px; border-bottom:1px solid #f0f2f0; }
.insight-book-row > div,.bookmark-copy { min-width:0; display:flex; flex-direction:column; gap:4px; }
.insight-book-row strong,.bookmark-copy strong { overflow:hidden; color:#4c5a50; font-size:14px; text-overflow:ellipsis; white-space:nowrap; }
.insight-book-row small,.bookmark-copy small { color:#65736a; font-size:14px; }
.insight-book-row > span { color:#758279; font-size:13px; }
.bookmark-list { display:grid; gap:3px; margin-top:17px; }
.bookmark-row { display:flex; align-items:center; justify-content:space-between; gap:7px; padding:8px 4px; border-bottom:1px solid #f0f2f0; }
.bookmark-open { min-width:0; min-height:48px; flex:1; display:flex; align-items:center; gap:10px; padding:4px 0; border:0; background:transparent; text-align:left; cursor:pointer; }
.bookmark-open:disabled { cursor:not-allowed; opacity:.74; }
.bookmark-copy .bookmark-orphaned-label { color:#955d4e; }
.bookmark-mark { color:#b58c58; font-size:18px; line-height:1; }
.bookmark-copy em { max-width:100%; overflow:hidden; color:#59675e; font-size:14px; font-style:normal; text-overflow:ellipsis; white-space:nowrap; }
.count-pill { min-width:25px; padding:5px 7px; border-radius:12px; color:#67836f; background:#edf4ef; font-size:13px; text-align:center; }
.insight-empty { display:grid; min-height:160px; place-items:center; padding:20px; color:#9ba49e; font-size:14px; line-height:1.7; text-align:center; }
@media(max-width:760px) { .insights-grid { grid-template-columns:1fr; } .insight-card { padding:16px 13px; } }
</style>
