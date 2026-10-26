<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import type { BookResource, ChapterResource } from "../../api/types";
import { useVirtualList } from "../../ui/useVirtualList";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";

const props = defineProps<{
  book: BookResource;
  currentIndex?: number;
  busy?: boolean;
  locked?: boolean;
  canDownload?: boolean;
  canReplace?: boolean;
  autoLocate?: boolean;
}>();
const emit = defineEmits<{ select: [index: number]; download: []; replace: [chapter: ChapterResource] }>();
const query = ref("");
const descending = ref(false);
const viewport = ref<HTMLElement | null>(null);
const activeIndex = computed(() => props.currentIndex ?? (props.book.progress?.updatedAtMs ? props.book.progress.chapterIndex : undefined));
const chapters = computed(() => {
  const keyword = query.value.trim().toLocaleLowerCase();
  const values = keyword ? props.book.chapters.filter(chapter => `${chapter.index + 1} ${chapter.title}`.toLocaleLowerCase().includes(keyword)) : props.book.chapters;
  return descending.value ? values.slice().reverse() : values;
});
const windowed = useVirtualList(chapters, viewport, 56);
const { range, visibleItems } = windowed;
const searchPlaceholder = computed(() => props.book.contentType === "pdf" ? "搜索页码或页名" : props.book.mediaType === "video" ? "搜索剧集" : props.book.mediaType === "audio" ? "搜索节目" : "搜索章节");
async function locateCurrent(): Promise<void> {
  query.value = "";
  await nextTick();
  windowed.scrollToIndex(chapters.value.findIndex(chapter => chapter.index === activeIndex.value));
}
watch([query, descending], () => { void windowed.reset(); });
watch(() => props.book.id, () => { query.value = ""; descending.value = false; void windowed.reset(); });
onMounted(() => { if (props.autoLocate) void locateCurrent(); });
</script>

<template>
  <section class="book-catalog-view">
    <div class="catalog-tools">
      <label class="catalog-search">
        <PrototypeIcon name="search" aria-hidden="true" />
        <input v-model="query" type="search" :placeholder="searchPlaceholder" :aria-label="searchPlaceholder" />
        <button v-if="query" type="button" aria-label="清空目录搜索" @click="query = ''"><PrototypeIcon name="close" /></button>
      </label>
      <div class="catalog-tools-actions">
        <button type="button" title="定位当前" aria-label="定位当前" :disabled="activeIndex === undefined" @click="locateCurrent"><PrototypeIcon name="tune" /><span>定位当前</span></button>
        <button type="button" :title="descending ? '切换为正序' : '切换为倒序'" :aria-label="descending ? '切换为正序' : '切换为倒序'" :aria-pressed="descending" @click="descending = !descending"><PrototypeIcon name="filter" /><span>{{ descending ? '倒序' : '正序' }}</span></button>
        <button type="button" title="缓存全部" aria-label="缓存全部" :disabled="!canDownload || locked || busy" @click="emit('download')"><PrototypeIcon name="download" /><span>缓存全部</span></button>
      </div>
    </div>
    <div ref="viewport" class="catalog-scroll">
      <div :ref="windowed.list" class="catalog-list">
        <div aria-hidden="true" :style="{ height: `${range.top}px` }"></div>
        <article v-for="chapter in visibleItems" :key="chapter.id" class="catalog-row-wrap" :class="{ volume: chapter.volume, 'can-replace': canReplace }">
          <button type="button" class="catalog-row" :data-chapter-index="chapter.index" :disabled="busy || locked || chapter.volume" :aria-current="chapter.index === activeIndex ? 'true' : undefined" @click="emit('select', chapter.index)">
            <strong>{{ chapter.title }}</strong>
            <span v-if="chapter.index === activeIndex" class="catalog-current-label">当前</span>
            <span v-if="!chapter.volume && !chapter.resource && !chapter.pages?.length && !chapter.media" class="catalog-uncached" title="未缓存" aria-label="未缓存"><PrototypeIcon name="cloud" /></span>
          </button>
          <button v-if="canReplace && !chapter.volume" type="button" class="catalog-replace-action" :disabled="busy || locked" :aria-label="`更换《${chapter.title}》的章节来源`" title="更换章节来源" @click="emit('replace', chapter)"><PrototypeIcon name="source-switch" /></button>
        </article>
        <div aria-hidden="true" :style="{ height: `${range.bottom}px` }"></div>
        <div v-if="!book.chapters.length" class="page-empty-state">暂无可显示的目录。</div>
        <div v-else-if="!chapters.length" class="page-empty-state">没有匹配的内容。<button type="button" class="text-button" @click="query = ''">清空搜索</button></div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.book-catalog-view { display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; color: var(--app-text); background: var(--app-background); }
.catalog-tools { display: grid; flex: none; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--app-line); }
.catalog-search { display: flex; align-items: center; gap: 6px; min-width: 0; height: 34px; padding-inline: 8px 4px; border: 1px solid var(--app-line); border-radius: var(--ui-radius); color: var(--app-muted); background: var(--app-surface); }
.catalog-search > .prototype-icon { width: 18px; height: 18px; }
.catalog-search input { flex: 1; width: 0; min-width: 0; padding: 0; border: 0; outline: 0; color: var(--app-text); background: transparent; font-size: var(--ui-font-body); }
.catalog-search input::-webkit-search-cancel-button { appearance: none; }
.catalog-search:focus-within { border-color: var(--app-accent); background: var(--app-surface); }
.catalog-search button { display: grid; flex: none; place-items: center; width: 32px; height: 32px; color: var(--app-muted); }
.catalog-search button .prototype-icon { width: 16px; height: 16px; }
.catalog-tools-actions { display: flex; gap: 4px; }
.catalog-tools-actions button { display: inline-flex; align-items: center; justify-content: center; gap: 4px; min-height: 32px; padding-inline: 8px; border-radius: var(--ui-radius); color: var(--app-muted); background: transparent; font-size: var(--ui-font-caption); white-space: nowrap; }
.catalog-tools-actions .prototype-icon { width: 16px; height: 16px; }
.catalog-scroll { flex: 1; min-height: 0; overflow: auto; overscroll-behavior: contain; scrollbar-width: thin; }
.catalog-row-wrap { display: grid; position: relative; grid-template-columns: minmax(0, 1fr); height: 56px; border-bottom: 1px solid var(--app-line); }
.catalog-row-wrap.can-replace { grid-template-columns: minmax(0, 1fr) 40px; }
.catalog-row { display: flex; align-items: center; gap: 8px; width: 100%; height: 55px; min-width: 0; padding-inline: 10px; color: var(--app-text); text-align: left; }
.catalog-row strong { flex: 1; min-width: 0; overflow: hidden; font-size: var(--ui-font-body); font-weight: 400; text-overflow: ellipsis; white-space: nowrap; }
.catalog-row-wrap.volume strong { color: var(--app-muted); font-weight: 600; }
.catalog-current-label { flex: none; color: var(--app-muted); font-size: var(--ui-font-caption); }
.catalog-uncached { display: grid; flex: none; place-items: center; color: var(--app-muted); }
.catalog-uncached .prototype-icon { width: 16px; height: 16px; }
.catalog-replace-action { display: grid; align-self: center; place-items: center; width: 40px; height: 44px; border-radius: 8px; color: var(--app-muted); }
.catalog-replace-action .prototype-icon { width: 18px; height: 18px; }
button { cursor: pointer; }
button:disabled { cursor: default; }
.catalog-row:focus-visible { outline-offset: -2px; }
@media (max-width: 680px) {
  .catalog-tools-actions button { width: 32px; padding: 0; }
  .catalog-tools-actions span { display: none; }
}
</style>
