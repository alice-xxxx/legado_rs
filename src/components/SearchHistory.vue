<script setup lang="ts">
import { computed, ref } from "vue";

interface SearchHistoryEntry {
  query: string;
  normalizedQuery: string;
  usage: number;
  firstUseTimeMs: number;
  lastUseTimeMs: number;
}

const props = withDefaults(
  defineProps<{
    entries: SearchHistoryEntry[];
    busy?: boolean;
    error?: string;
  }>(),
  { busy: false, error: "" },
);

const emit = defineEmits<{
  useQuery: [query: string];
  deleteQuery: [query: string];
  clearAll: [];
}>();

const sortMode = ref<"recent" | "popular">("recent");
const clearConfirmOpen = ref(false);

const sortedEntries = computed(() => [...props.entries].sort((left, right) => {
  const recency = right.lastUseTimeMs - left.lastUseTimeMs;
  if (sortMode.value === "popular") return right.usage - left.usage || recency || left.query.localeCompare(right.query, "zh-CN");
  return recency || right.usage - left.usage || left.query.localeCompare(right.query, "zh-CN");
}));

function formatLastUsed(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "使用时间未知";
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "使用时间未知";
  const now = new Date();
  const isToday = date.getFullYear() === now.getFullYear()
    && date.getMonth() === now.getMonth()
    && date.getDate() === now.getDate();
  const time = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(date);
  if (isToday) return `今天 ${time}`;
  return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(date);
}

function clearAll(): void {
  if (props.busy || !props.entries.length) return;
  clearConfirmOpen.value = false;
  emit("clearAll");
}
</script>

<template>
  <section class="search-history" aria-labelledby="search-history-title" data-testid="search-history">
    <header class="search-history__header">
      <div>
        <p class="search-history__eyebrow">搜索</p>
        <h2 id="search-history-title">搜索历史</h2>
        <p class="search-history__description">点选记录可再次搜索。</p>
      </div>
      <button
        type="button"
        class="search-history__button search-history__button--clear"
        data-testid="search-history-clear"
        :disabled="busy || !entries.length"
        @click="clearConfirmOpen = true"
      >清空历史</button>
    </header>

    <div class="search-history__toolbar">
      <div class="search-history__sort" role="group" aria-label="搜索历史排序">
        <button type="button" data-testid="search-history-sort-recent" :aria-pressed="sortMode === 'recent'" :disabled="busy" @click="sortMode = 'recent'">最近使用</button>
        <button type="button" data-testid="search-history-sort-popular" :aria-pressed="sortMode === 'popular'" :disabled="busy" @click="sortMode = 'popular'">常用搜索</button>
      </div>
      <span class="search-history__count">{{ entries.length }} 条记录</span>
    </div>

    <p v-if="error" class="search-history__error" role="alert" data-testid="search-history-error">{{ error }}</p>

    <div v-if="entries.length" class="search-history__list" aria-label="搜索词列表">
      <article
        v-for="(entry, index) in sortedEntries"
        :key="entry.normalizedQuery"
        class="search-history__entry"
        :data-testid="`search-history-entry-${index}`"
        :data-query="entry.query"
      >
        <button
          type="button"
          class="search-history__query"
          :data-testid="`search-history-use-${index}`"
          :disabled="busy"
          @click="emit('useQuery', entry.query)"
        >
          <span class="search-history__magnifier" aria-hidden="true">⌕</span>
          <span class="search-history__query-copy">
            <strong>{{ entry.query }}</strong>
            <small>{{ entry.usage }} 次搜索 · 最近 {{ formatLastUsed(entry.lastUseTimeMs) }}</small>
          </span>
        </button>
        <button
          type="button"
          class="search-history__delete"
          :data-testid="`search-history-delete-${index}`"
          :aria-label="`删除搜索词：${entry.query}`"
          :disabled="busy"
          @click="emit('deleteQuery', entry.query)"
        >删除</button>
      </article>
    </div>

    <div v-else class="search-history__empty" data-testid="search-history-empty">
      <span class="search-history__empty-icon" aria-hidden="true">⌕</span>
      <strong>还没有搜索记录</strong>
      <p>搜索过的书名和关键词会显示在这里。</p>
    </div>

    <section v-if="clearConfirmOpen" class="search-history__confirm-backdrop" aria-label="确认清空搜索历史" @click.self="clearConfirmOpen = false">
      <div class="search-history__confirm" role="alertdialog" aria-modal="true" aria-labelledby="search-history-confirm-title" aria-describedby="search-history-confirm-description">
        <h3 id="search-history-confirm-title">清空搜索历史？</h3>
        <p id="search-history-confirm-description">将删除全部 {{ entries.length }} 条搜索记录。此操作无法撤销。</p>
        <div class="search-history__confirm-actions">
          <button type="button" class="search-history__button" data-testid="search-history-clear-cancel" :disabled="busy" @click="clearConfirmOpen = false">保留记录</button>
          <button type="button" class="search-history__button search-history__button--danger" data-testid="search-history-clear-confirm" :disabled="busy" @click="clearAll">{{ busy ? '正在清空…' : '确认清空' }}</button>
        </div>
      </div>
    </section>
  </section>
</template>

<style scoped>
.search-history { min-width:0; display:flex; flex-direction:column; gap:14px; padding:18px; border:1px solid #e1e8e1; border-radius:13px; color:#36463a; background:#fff; }
.search-history__header { display:flex; align-items:flex-start; justify-content:space-between; gap:14px; }
.search-history__eyebrow { margin:0 0 3px; color:#738c77; font-size:12px; font-weight:700; letter-spacing:.08em; }
.search-history__header h2 { margin:0; font-size:19px; line-height:1.35; }
.search-history__description { margin:4px 0 0; color:#728076; font-size:14px; }
.search-history__button { min-height:44px; padding:0 14px; border:1px solid #dce5dc; border-radius:8px; color:#435649; background:#fff; font:inherit; font-size:14px; cursor:pointer; }
.search-history__button:disabled,.search-history__sort button:disabled,.search-history__query:disabled,.search-history__delete:disabled { opacity:.5; cursor:not-allowed; }
.search-history__button--clear { flex:none; }
.search-history__toolbar { min-height:44px; display:flex; align-items:center; justify-content:space-between; gap:12px; padding-bottom:8px; border-bottom:1px solid #edf0ed; }
.search-history__sort { display:flex; align-items:center; gap:4px; padding:3px; border:1px solid #e4eae4; border-radius:9px; background:#f6f8f5; }
.search-history__sort button { min-height:36px; padding:0 12px; border:0; border-radius:7px; color:#68766b; background:transparent; font:inherit; font-size:14px; cursor:pointer; }
.search-history__sort button[aria-pressed="true"] { color:#3d6548; background:#fff; box-shadow:0 1px 3px #26382a18; font-weight:700; }
.search-history__count { color:#7a867d; font-size:13px; white-space:nowrap; }
.search-history__error { margin:0; padding:10px 12px; border-radius:8px; color:#844d45; background:#fff3f1; font-size:14px; line-height:1.5; }
.search-history__list { min-height:0; max-height:520px; overflow:auto; }
.search-history__entry { min-height:64px; display:flex; align-items:center; gap:10px; border-bottom:1px solid #edf0ed; }
.search-history__entry:last-child { border-bottom:0; }
.search-history__query { min-width:0; min-height:58px; flex:1; display:flex; align-items:center; gap:11px; padding:6px 4px; border:0; color:inherit; background:transparent; text-align:left; cursor:pointer; }
.search-history__query:hover .search-history__query-copy strong { color:#3d754d; }
.search-history__magnifier { width:34px; height:34px; flex:none; display:grid; place-items:center; border-radius:50%; color:#62806a; background:#f0f5ef; font-size:19px; }
.search-history__query-copy { min-width:0; display:flex; flex-direction:column; gap:4px; }
.search-history__query-copy strong { overflow:hidden; color:#3c4b40; font-size:15px; font-weight:600; text-overflow:ellipsis; white-space:nowrap; }
.search-history__query-copy small { color:#76837a; font-size:13px; }
.search-history__delete { min-width:58px; min-height:44px; border:0; border-radius:7px; color:#8a5b54; background:transparent; font:inherit; font-size:14px; cursor:pointer; }
.search-history__delete:hover:not(:disabled) { background:#fff4f2; }
.search-history__empty { min-height:180px; display:flex; align-items:center; justify-content:center; flex-direction:column; gap:7px; color:#6b7b70; text-align:center; }
.search-history__empty-icon { width:42px; height:42px; display:grid; place-items:center; margin-bottom:3px; border-radius:50%; color:#62806a; background:#f0f5ef; font-size:23px; }
.search-history__empty strong { color:#45564a; font-size:15px; }
.search-history__empty p { margin:0; color:#7a877e; font-size:14px; }
.search-history__confirm-backdrop { position:fixed; z-index:1; inset:0; display:grid; place-items:center; padding:16px; background:#1e2a2259; }
.search-history__confirm { width:min(390px,100%); padding:20px; border:1px solid #e2e8e2; border-radius:13px; color:#36463a; background:#fff; box-shadow:0 16px 44px #17251b38; }
.search-history__confirm h3 { margin:0; font-size:18px; }
.search-history__confirm p { margin:9px 0 18px; color:#69766d; font-size:14px; line-height:1.55; }
.search-history__confirm-actions { display:flex; justify-content:flex-end; gap:8px; }
.search-history__button--danger { border-color:#e9d4d0; color:#8e4f47; background:#fff6f4; }
@media(max-width:560px) {
  .search-history { padding:14px; }
  .search-history__header { align-items:center; }
  .search-history__header h2 { font-size:18px; }
  .search-history__button--clear { min-height:44px; padding:0 10px; }
  .search-history__toolbar { align-items:flex-start; flex-direction:column; gap:7px; }
  .search-history__sort { width:100%; }
  .search-history__sort button { flex:1; }
  .search-history__entry { gap:5px; }
  .search-history__query-copy strong { font-size:14px; }
  .search-history__query-copy small { font-size:12px; }
}
</style>
