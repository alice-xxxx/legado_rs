<script setup lang="ts">
// 搜索历史只负责排序、确认和展示；记录的读取与保存由发现页处理。
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

function clearAll(): void {
  if (props.busy || !props.entries.length) return;
  clearConfirmOpen.value = false;
  emit("clearAll");
}
</script>

<template>
  <section class="search-history">
    <header class="search-history__header">
      <div>
        <h2 id="search-history-title">搜索历史</h2>
      </div>
      <button
        type="button"
        class="search-history__button search-history__button--clear"
        :disabled="busy || !entries.length"
        @click="clearConfirmOpen = true"
      >清空历史</button>
    </header>

    <div class="search-history__toolbar">
      <div class="search-history__sort">
        <button type="button" :class="{ active: sortMode === 'recent' }" :disabled="busy" @click="sortMode = 'recent'">最近使用</button>
        <button type="button" :class="{ active: sortMode === 'popular' }" :disabled="busy" @click="sortMode = 'popular'">常用搜索</button>
      </div>
      <span class="search-history__count">{{ entries.length }} 条记录</span>
    </div>

    <p v-if="error" class="search-history__error">{{ error }}</p>

    <div v-if="entries.length" class="search-history__list">
      <article
        v-for="entry in sortedEntries"
        :key="entry.normalizedQuery"
        class="search-history__entry"
      >
        <button
          type="button"
          class="search-history__query"
          :disabled="busy"
          @click="emit('useQuery', entry.query)"
        >
          <span class="search-history__query-copy">
            <strong>{{ entry.query }}</strong>
          </span>
        </button>
        <button
          type="button"
          class="search-history__delete"
          :disabled="busy"
          @click="emit('deleteQuery', entry.query)"
        >删除</button>
      </article>
    </div>

    <section v-if="clearConfirmOpen" class="search-history__confirm-backdrop" @click.self="clearConfirmOpen = false">
      <div class="search-history__confirm">
        <h3 id="search-history-confirm-title">清空搜索历史？</h3>
        <p id="search-history-confirm-description">将删除全部 {{ entries.length }} 条搜索记录。此操作无法撤销。</p>
        <div class="search-history__confirm-actions">
          <button type="button" class="search-history__button" :disabled="busy" @click="clearConfirmOpen = false">保留记录</button>
          <button type="button" class="search-history__button search-history__button--danger" :disabled="busy" @click="clearAll">{{ busy ? '正在清空…' : '确认清空' }}</button>
        </div>
      </div>
    </section>
  </section>
</template>

<style scoped>
.search-history {
  min-width: 0;
  margin: 8px 0 14px;
  color: var(--app-text);
}

.search-history__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
}

.search-history__header h2 {
  margin: 0;
  color: var(--app-text);
  font-size: 14px;
  font-weight: 700;
  line-height: 1.3;
}

.search-history__button {
  min-height: 34px;
  padding: 0 11px;
  border: 1px solid var(--app-line);
  border-radius: 9px;
  color: var(--app-text);
  background: var(--app-surface);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.search-history__button:disabled,
.search-history__sort button:disabled,
.search-history__query:disabled,
.search-history__delete:disabled {
  opacity: .5;
  cursor: not-allowed;
}

.search-history__button--clear {
  flex: none;
}

.search-history__toolbar {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  margin-top: 5px;
}

.search-history__sort {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
  background: var(--app-panel-soft);
}

.search-history__sort button {
  min-height: 29px;
  padding: 0 10px;
  border: 0;
  border-radius: 7px;
  color: var(--app-muted);
  background: transparent;
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.search-history__sort button.active {
  color: var(--app-accent-ink);
  background: var(--app-surface);
  box-shadow: 0 1px 3px rgb(30 33 45 / 8%);
  font-weight: 700;
}

.search-history__count {
  color: var(--app-muted);
  font-size: 11px;
  white-space: nowrap;
}

.search-history__error {
  margin: 6px 0 0;
  padding: 8px 10px;
  border-radius: 9px;
  color: var(--app-danger);
  background: #fff3f4;
  font-size: 12px;
  line-height: 1.4;
}

.search-history__list {
  display: flex;
  max-height: 76px;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 6px;
  overflow: auto;
}

.search-history__entry {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 4px;
  padding: 2px 4px 2px 9px;
  border: 1px solid var(--app-line);
  border-radius: 9px;
  background: var(--app-surface);
}

.search-history__query {
  display: flex;
  min-width: 0;
  max-width: min(210px, 55vw);
  min-height: 27px;
  align-items: center;
  gap: 5px;
  padding: 2px 0;
  border: 0;
  color: inherit;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.search-history__query:hover .search-history__query-copy strong {
  color: var(--app-accent-ink);
}

.search-history__query-copy {
  min-width: 0;
}

.search-history__query-copy strong {
  overflow: hidden;
  color: var(--app-text);
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-history__delete {
  min-width: 26px;
  min-height: 26px;
  padding: 0 5px;
  border: 0;
  border-radius: 6px;
  color: var(--app-danger);
  background: transparent;
  font: inherit;
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
}

.search-history__delete:hover:not(:disabled) {
  background: #fff3f4;
}

.search-history__confirm-backdrop {
  position: fixed;
  z-index: 60;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 16px;
  background: rgb(24 25 29 / 36%);
}

.search-history__confirm {
  width: min(390px, 100%);
  padding: 20px;
  border: 1px solid var(--app-line);
  border-radius: 16px;
  color: var(--app-text);
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
}

.search-history__confirm h3 {
  margin: 0;
  font-size: 16px;
}

.search-history__confirm p {
  margin: 9px 0 18px;
  color: var(--app-muted);
  font-size: 13px;
  line-height: 1.55;
}

.search-history__confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.search-history__button--danger {
  border-color: #e7c8cc;
  color: var(--app-danger);
  background: #fff7f8;
}

@media (max-width: 560px) {
  .search-history__sort button {
    padding: 0 7px;
  }

  .search-history__query {
    max-width: min(170px, 52vw);
  }
}
</style>
