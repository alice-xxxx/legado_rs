<script setup lang="ts">
import { computed, onBeforeUnmount, onBeforeUpdate, onMounted, onUpdated, ref } from "vue";
import { settings } from "../settings/settingsStore";
import NovelChapter from "./NovelChapter.vue";
import type { ReaderTextChapter } from "./readerContent";
const props = defineProps<{ text?: string; markup?: string; bookId: string; title: string; chapterIndex: number; paragraphIndent?: boolean; chapters?: ReaderTextChapter[] }>();
const emit = defineEmits<{ ready: [element: HTMLElement | null]; layout: []; textChange: [] }>();
const viewport = ref<HTMLElement | null>(null);
const visibleChapters = computed<ReaderTextChapter[]>(() => props.chapters?.length ? props.chapters : [{
  index: props.chapterIndex, title: props.title, content: props.text != null
    ? { schemaVersion: 1, kind: "text", text: props.text }
    : { schemaVersion: 1, kind: "richText", markup: props.markup ?? "" }
}]);
const readerStyle = computed(() => ({
  "--novel-font-size": `${settings.value.reader.fontSizePx}px`,
  "--novel-line-height": settings.value.reader.lineHeight,
  "--novel-font-family": settings.value.reader.fontFamily || "system-ui",
  "--novel-text-align": settings.value.reader.textAlign,
  "--novel-background-image": settings.value.reader.backgroundImageSrc
    ? `url(${JSON.stringify(settings.value.reader.backgroundImageSrc)})` : "none",
  "--novel-background-opacity": settings.value.reader.backgroundImageOpacity / 100,
}));
let lastChapters = visibleChapters.value, lastStyle = readerStyle.value, lastMode = settings.value.reader.readingMode;
let layoutChanged = false;
let anchor: { index: string; top: number } | null = null;
let resizeObserver: ResizeObserver | undefined;
onMounted(() => {
  emit("ready", viewport.value);
  resizeObserver = new ResizeObserver(() => emit("layout"));
  if (viewport.value) resizeObserver.observe(viewport.value);
  emit("layout");
});
onBeforeUpdate(() => {
  layoutChanged = lastChapters !== visibleChapters.value || lastStyle !== readerStyle.value || lastMode !== settings.value.reader.readingMode;
  anchor = null;
  if (layoutChanged && settings.value.reader.readingMode === "vertical" && viewport.value) {
    const top = viewport.value.getBoundingClientRect().top;
    const article = [...viewport.value.querySelectorAll<HTMLElement>(".chapter-content")].find(article => article.getBoundingClientRect().bottom > top);
    if (article?.dataset.readerChapter) anchor = { index: article.dataset.readerChapter, top: article.getBoundingClientRect().top };
  }
  if (lastChapters.some(chapter => visibleChapters.value.some(next => next.index === chapter.index && next.content !== chapter.content))) emit("textChange");
});
onUpdated(() => {
  if (anchor && viewport.value) {
    const article = viewport.value.querySelector<HTMLElement>(`.chapter-content[data-reader-chapter="${anchor.index}"]`);
    if (article) viewport.value.scrollTop += article.getBoundingClientRect().top - anchor.top;
  }
  lastChapters = visibleChapters.value; lastStyle = readerStyle.value; lastMode = settings.value.reader.readingMode;
  if (layoutChanged) emit("layout");
});
onBeforeUnmount(() => { resizeObserver?.disconnect(); emit("ready", null); });
</script>
<template>
  <div ref="viewport" class="novel-reader" :data-reader-chapter-index="chapterIndex" :class="{ vertical: settings.reader.readingMode === 'vertical', 'vertical-turn': settings.reader.readingMode === 'slide-vertical' }" :style="readerStyle" @load.capture="emit('layout')">
    <NovelChapter v-for="chapter in visibleChapters" :key="chapter.index" :index="chapter.index" :title="chapter.title" :book-id="bookId" :paragraph-indent="paragraphIndent"
      :text="chapter.content.kind === 'text' ? chapter.content.text : undefined" :markup="chapter.content.kind === 'richText' ? chapter.content.markup : undefined" />
  </div>
</template>
<style scoped>
.novel-reader {
  --gutter: min(8vw, 96px);
  position: relative;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  padding: var(--reader-text-top-inset, 0px) 0 var(--reader-text-bottom-inset, 0px);
  overflow: hidden;
  column-count: 1;
  column-gap: 0;
  column-fill: auto;
  color: var(--reader-text);
  background-color: var(--reader-background);
  font-family: var(--novel-font-family);
  font-size: var(--novel-font-size);
  line-height: var(--novel-line-height);
  text-align: var(--novel-text-align);
  touch-action: pan-y;
  scrollbar-width: none;
  overscroll-behavior: contain;
  isolation: isolate;
}
.novel-reader::before {
  content: "";
  position: absolute;
  inset: 0;
  background-image: var(--novel-background-image);
  background-position: center;
  background-size: cover;
  opacity: var(--novel-background-opacity);
  z-index: -1;
  pointer-events: none;
}
.novel-reader.vertical { overflow-y: auto; column-count: auto; overflow-anchor: none; }
.novel-reader.vertical :deep(.chapter-content) { display: flow-root; padding-bottom: 2em; }
.reader-chapter-preview { position: absolute; inset: 0; visibility: hidden; pointer-events: none; }
.novel-reader.vertical-turn { touch-action: pan-x; }
</style>
