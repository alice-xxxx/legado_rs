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
        <PrototypeIcon name="search" />
        <input v-model="query" type="search" :placeholder="searchPlaceholder" :aria-label="searchPlaceholder" />
        <button v-if="query" type="button" aria-label="清空目录搜索" @click="query = ''"><PrototypeIcon name="close" /></button>
      </label>
      <div class="catalog-tools-actions">
        <button type="button" :disabled="activeIndex === undefined" @click="locateCurrent"><PrototypeIcon name="tune" /><span>定位当前</span></button>
        <button type="button" :aria-pressed="descending" @click="descending = !descending"><PrototypeIcon name="filter" /><span>{{ descending ? '倒序' : '正序' }}</span></button>
        <button type="button" :disabled="!canDownload || locked || busy" @click="emit('download')"><PrototypeIcon name="download" /><span>缓存全部</span></button>
      </div>
    </div>
    <div ref="viewport" class="catalog-scroll">
      <div :ref="windowed.list" class="catalog-list">
        <div aria-hidden="true" :style="{ height: `${range.top}px` }"></div>
        <article v-for="chapter in visibleItems" :key="chapter.id" class="catalog-row-wrap" :class="{ current: chapter.index === activeIndex, volume: chapter.volume, 'can-replace': canReplace }">
          <button type="button" class="catalog-row" :data-chapter-index="chapter.index" :disabled="busy || locked || chapter.volume" :aria-current="chapter.index === activeIndex ? 'true' : undefined" @click="emit('select', chapter.index)">
            <strong>{{ chapter.title }}</strong>
            <span v-if="chapter.index === activeIndex" class="catalog-current-label">当前</span>
            <span v-if="!chapter.resource && !chapter.volume" class="catalog-uncached" title="正文尚未准备，打开时读取"><PrototypeIcon name="cloud" /></span>
          </button>
          <button v-if="canReplace && !chapter.volume" type="button" class="catalog-replace-action" :disabled="busy || locked" :aria-label="`更换《${chapter.title}》的章节来源`" title="更换章节来源" @click="emit('replace', chapter)"><PrototypeIcon name="source-switch" /></button>
        </article>
        <div aria-hidden="true" :style="{ height: `${range.bottom}px` }"></div>
        <p v-if="!book.chapters.length" class="catalog-empty">暂无可显示的目录。</p>
        <p v-else-if="!chapters.length" class="catalog-empty">没有匹配的内容。<button type="button" class="text-button" @click="query = ''">清空搜索</button></p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.book-catalog-view { display: flex; height: 100%; min-height: 0; flex-direction: column; color: var(--app-text); background: var(--app-background); }
.catalog-tools { display: grid; flex: none; grid-template-columns: minmax(200px, 300px) minmax(0, 1fr); gap: 12px; padding: 10px 0 12px; border-bottom: 1px solid var(--app-line); }
.catalog-search { display: flex; min-width: 0; min-height: 36px; align-items: center; gap: 7px; padding: 0 9px; border: 1px solid var(--app-line); border-radius: 10px; color: var(--app-muted); background: var(--app-surface); }
.catalog-search input { width: 100%; min-width: 0; border: 0; outline: none; color: var(--app-text); background: transparent; font-size: var(--ui-font-body); }
.catalog-search:focus-within { outline: 2px solid var(--app-accent); outline-offset: 2px; }
.catalog-search button { display: grid; width: 30px; height: 30px; flex: none; place-items: center; border: 0; background: transparent; color: var(--app-muted); }
.catalog-tools-actions { display: flex; gap: 6px; }
.catalog-tools-actions button { display: inline-flex; min-height: 36px; flex: 1; align-items: center; justify-content: center; gap: 5px; padding: 0 8px; border: 0; border-radius: 8px; color: var(--app-muted); background: var(--app-panel-soft); white-space: nowrap; font-size: var(--ui-font-body); }
.catalog-scroll { min-height: 0; flex: 1; overflow: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }
.catalog-row-wrap { display: grid; position: relative; height: 56px; grid-template-columns: minmax(0, 1fr); align-items: center; gap: 4px; border-bottom: 1px solid var(--app-line); }
.catalog-row-wrap.can-replace { grid-template-columns: minmax(0, 1fr) 34px; }
.catalog-row { display: flex; width: 100%; height: 55px; min-width: 0; align-items: center; gap: 8px; padding: 0 6px; border: 0; border-radius: 0; color: var(--app-text); background: transparent; text-align: left; }
.catalog-row strong { min-width: 0; flex: 1; overflow: hidden; font-size: var(--ui-font-caption); font-weight: 500; text-overflow: ellipsis; white-space: nowrap; }
.catalog-row-wrap.current .catalog-row { background: var(--app-accent-soft); }
.catalog-row-wrap.current::before { position: absolute; top: 12px; bottom: 12px; left: 0; width: 2px; border-radius: 2px; background: var(--app-accent); content: ""; }
.catalog-row-wrap.volume strong { color: var(--app-muted); font-weight: 700; }
.catalog-current-label { flex: none; color: var(--app-accent-ink); font-size: var(--ui-font-caption); }
.catalog-uncached { display: grid; width: 20px; flex: none; place-items: center; color: var(--app-muted); }
.catalog-uncached .icon { width: 15px; height: 15px; }
.catalog-replace-action { display: grid; width: 30px; height: 34px; place-items: center; border: 0; border-radius: 8px; color: var(--app-muted); background: transparent; }
.catalog-empty { padding: 30px 8px; color: var(--app-muted); text-align: center; }
button { cursor: pointer; }
button:disabled { cursor: default; opacity: .5; }
button:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
@media (max-width: 680px) { .catalog-tools { grid-template-columns: minmax(0, 1fr); gap: 8px; } }
</style>
