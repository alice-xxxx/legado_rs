<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton, Button as KButton } from "konsta/vue";
import { computed, ref } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import SelectMenu from "../../ui/SelectMenu.vue";
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
      <SelectMenu v-model="sortMode" label="历史排序" :disabled="busy"
        :options="[{ value: 'recent', label: '最近' }, { value: 'popular', label: '常用' }]" />
      <KButton rounded inline clear type="button" class="search-history__clear gap-2" :disabled="busy || !entries.length"
        @click="clearConfirmOpen = true">清空</KButton>
    </header>
    <div class="search-history__list">
      <span v-for="entry in sortedEntries" :key="entry.normalizedQuery" class="search-history__entry">
        <KButton rounded inline clear type="button" class="search-history__query gap-2" :disabled="busy" :title="entry.query"
          @click="emit('useQuery', entry.query)">{{ entry.query }}</KButton>
        <KButton rounded inline clear type="button" class="search-history__delete k-color-brand-danger gap-2 " :disabled="busy"
          :aria-label="'删除搜索记录：' + entry.query" @click="emit('deleteQuery', entry.query)">
          <PrototypeIcon name="close" />
        </KButton>
      </span>
    </div>
    <p v-if="error" class="search-history__error">{{ error }}</p>
    <section v-if="clearConfirmOpen" class="search-history__confirm-backdrop" @click.self="clearConfirmOpen = false">
      <KDialog title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]" opened :backdrop="false" class="search-history__confirm w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="search-history-confirm-title">清空搜索历史？</h3></template>
<p id="search-history-confirm-description">将删除全部 {{ entries.length }} 条搜索记录。</p><template #buttons><KDialogButton type="button" data-back-action :disabled="busy" @click="clearConfirmOpen = false">保留记录</KDialogButton>
<KDialogButton type="button" :disabled="busy" @click="clearAll" strong>{{ busy ? '正在清空…' : '确认清空' }}</KDialogButton></template>
</KDialog>
    </section>
  </section>
</template>

<style scoped>
@layer components {
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
.search-history__clear {
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
.search-history__query {
  min-width: 0;
  max-width: min(240px, 65vw);
  min-height: 36px;
  overflow: hidden;
  padding: 0 4px 0 12px;
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
}
.search-history__delete svg {
  width: 12px;
  height: 12px;
}
.search-history__clear {
  min-height: 36px;
  padding: 0 4px;
  border: 0;
  background: transparent;
}
.search-history button:disabled {
  opacity: .5;
  cursor: not-allowed;
}
.search-history button:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}
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
  background: var(--app-scrim);
}
}
</style>
