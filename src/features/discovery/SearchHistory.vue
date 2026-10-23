<script setup lang="ts">
import { computed, ref } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import type { SearchHistoryEntry } from "../../api/types";

const props = withDefaults(defineProps<{
  entries: SearchHistoryEntry[];
  busy?: boolean;
  error?: string;
}>(), { busy: false, error: "" });

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
  <section class="search-history" aria-label="搜索历史">
    <header class="search-history__row">
      <h2 class="search-history__label">搜索历史</h2>
      <select v-model="sortMode" class="search-history__sort" aria-label="历史排序" :disabled="busy">
        <option value="recent">最近</option>
        <option value="popular">常用</option>
      </select>
      <button type="button" class="search-history__clear" :disabled="busy || !entries.length"
        @click="clearConfirmOpen = true">清空</button>
    </header>
    <div class="search-history__list">
      <span v-for="entry in sortedEntries" :key="entry.normalizedQuery" class="search-history__entry">
        <button type="button" class="search-history__query" :disabled="busy" :title="entry.query"
          @click="emit('useQuery', entry.query)">{{ entry.query }}</button>
        <button type="button" class="search-history__delete" :disabled="busy"
          :aria-label="'删除搜索记录：' + entry.query" @click="emit('deleteQuery', entry.query)">
          <PrototypeIcon name="close" />
        </button>
      </span>
    </div>
    <p v-if="error" class="search-history__error">{{ error }}</p>
    <section v-if="clearConfirmOpen" class="search-history__confirm-backdrop" @click.self="clearConfirmOpen = false">
      <div class="search-history__confirm" role="dialog" aria-modal="true"
        aria-labelledby="search-history-confirm-title" aria-describedby="search-history-confirm-description">
        <h3 id="search-history-confirm-title">清空搜索历史？</h3>
        <p id="search-history-confirm-description">将删除全部 {{ entries.length }} 条搜索记录。</p>
        <div class="search-history__confirm-actions">
          <button type="button" class="button secondary small" data-back-action :disabled="busy" @click="clearConfirmOpen = false">保留记录</button>
          <button type="button" class="button secondary small" :disabled="busy" @click="clearAll">{{ busy ? '正在清空…' : '确认清空' }}</button>
        </div>
      </div>
    </section>
  </section>
</template>

<style scoped>
.search-history {
  min-width: 0;
  padding-top: 20px;
  color: var(--app-text);
  font-size: var(--ui-font-caption);
}
.search-history__row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  min-height: 40px;
  margin-bottom: 8px;
}
.search-history__sort, .search-history__clear {
  flex: none;
  color: var(--app-muted);
}
.search-history__label {
  flex: 1;
  margin: 0;
  font-size: var(--ui-font-body);
  font-weight: 600;
}
.search-history__list {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.search-history__entry {
  display: flex;
  flex: none;
  align-items: center;
  max-width: 100%;
  border: 1px solid var(--app-line);
  border-radius: 8px;
  background: var(--app-panel-softer);
}
.search-history button, .search-history select {
  font: inherit;
  cursor: pointer;
}
.search-history__query {
  min-width: 0;
  max-width: min(240px, 65vw);
  min-height: 36px;
  overflow: hidden;
  padding: 0 4px 0 12px;
  border: 0;
  color: var(--app-text);
  background: transparent;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.search-history__delete {
  display: grid;
  place-items: center;
  flex: none;
  width: 32px;
  height: 36px;
  padding: 0;
  border: 0;
  border-radius: 5px;
  color: var(--app-muted);
  background: transparent;
}
.search-history__delete svg {
  width: 12px;
  height: 12px;
}
.search-history__delete:hover {
  color: var(--app-danger);
}
.search-history__sort, .search-history__clear {
  min-height: 36px;
  padding: 0 4px;
  border: 0;
  background: transparent;
}
.search-history button:disabled, .search-history select:disabled {
  opacity: .5;
  cursor: not-allowed;
}
.search-history button:focus-visible, .search-history select:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}
.search-history__entry:hover { border-color: var(--app-accent); }
.search-history__error {
  margin: 4px 0;
  color: var(--app-danger);
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
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
}
.search-history__confirm h3 {
  margin: 0;
  font-size: var(--ui-font-title);
}
.search-history__confirm p {
  margin: 9px 0 18px;
  color: var(--app-muted);
  line-height: 1.55;
}
.search-history__confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
