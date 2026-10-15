<script setup lang="ts">
import { computed, onBeforeUnmount, onBeforeUpdate, onMounted, onUpdated, ref } from "vue";
import { settings } from "../settings/settingsStore";
import { replacementRules } from "../settings/configurationStores";
import { applyTextRules, applyMarkupRules } from "./textReplacements";

const props = defineProps<{ text?: string; markup?: string; bookId: string; title: string; paragraphIndent?: boolean }>();
const emit = defineEmits<{
  ready: [element: HTMLElement | null];
  layout: [];
  textChange: [];
}>();
const viewport = ref<HTMLElement | null>(null);
const displayText = computed(() => applyTextRules(props.text ?? "", replacementRules.value, props.bookId));
function removeParagraphLeadingWhitespace(paragraph: HTMLElement): void {
  let atParagraphStart = true;
  const strip = (parent: Node) => {
    for (const child of Array.from(parent.childNodes)) {
      if (!atParagraphStart) break;
      if (child.nodeType === Node.TEXT_NODE) {
        const text = child as Text;
        text.data = text.data.replace(/^[\s\uFEFF]+/u, "");
        if (text.data.length) atParagraphStart = false;
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        const element = child as HTMLElement;
        if (element.matches("br,img,svg,video,audio,canvas,iframe,object,embed,input")) {
          atParagraphStart = false;
          break;
        }
        strip(element);
      }
    }
  };
  strip(paragraph);
}

const displayMarkup = computed(() => {
  const markup = applyMarkupRules(props.markup ?? "", replacementRules.value, props.bookId);
  if (!props.paragraphIndent) return markup;

  // Normalize source indentation once, then use one consistent CSS indent.
  const root = document.createElement("div");
  root.innerHTML = markup;
  root.querySelectorAll<HTMLElement>("p").forEach(paragraph => {
    paragraph.removeAttribute("data-reader-indent-normalized");
    if (paragraph.closest("pre,code")) return;
    removeParagraphLeadingWhitespace(paragraph);
    paragraph.setAttribute("data-reader-indent-normalized", "true");
  });
  return root.innerHTML;
});
const showTitle = computed(() => {
  if (props.text != null || !props.markup) return Boolean(props.title);
  const content = new DOMParser().parseFromString(displayMarkup.value, "text/html");
  const heading = content.body.querySelector("h1,h2");
  const normalize = (value: string) => value.replace(/\s+/g, " ").trim();
  return Boolean(props.title) && normalize(heading?.textContent ?? "") !== normalize(props.title);
});
const readerStyle = computed(() => ({
  "--novel-font-size": `${settings.value.reader.fontSizePx}px`,
  "--novel-line-height": settings.value.reader.lineHeight,
  "--novel-font-family": settings.value.reader.fontFamily || "system-ui",
  "--novel-text-align": settings.value.reader.textAlign,
  "--novel-background-image": settings.value.reader.backgroundImageSrc
    ? `url(${JSON.stringify(settings.value.reader.backgroundImageSrc)})` : "none",
  "--novel-background-opacity": settings.value.reader.backgroundImageOpacity / 100,
}));
let lastText = displayText.value + displayMarkup.value;
let resizeObserver: ResizeObserver | undefined;
onMounted(() => {
  emit("ready", viewport.value);
  resizeObserver = new ResizeObserver(() => emit("layout"));
  if (viewport.value) resizeObserver.observe(viewport.value);
  emit("layout");
});
onBeforeUpdate(() => {
  if (lastText !== displayText.value + displayMarkup.value) emit("textChange");
});
onUpdated(() => {
  lastText = displayText.value + displayMarkup.value;
  emit("layout");
});
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  emit("ready", null);
});
</script>

<template>
  <div ref="viewport" class="novel-reader" :class="{ vertical: settings.reader.readingMode === 'vertical', 'vertical-turn': settings.reader.readingMode === 'slide-vertical' }" :style="readerStyle" @load.capture="emit('layout')">
    <article class="chapter-content">
      <h1 v-if="showTitle" class="chapter-title">{{ title }}</h1>
      <div v-if="text != null" class="chapter-text">{{ displayText }}</div>
      <div v-else class="chapter-markup" :class="{ 'indent-paragraphs': paragraphIndent }" v-html="displayMarkup"></div>
    </article>
  </div>
</template>

<style scoped>
.novel-reader {
  --gutter: min(8vw, 96px);
  position: relative;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  padding: var(--reader-text-top-inset, 0px) var(--gutter) var(--reader-text-bottom-inset, 0px);
  overflow: hidden;
  column-count: 1;
  column-gap: calc(2 * var(--gutter));
  column-fill: auto;
  color: var(--reader-text);
  background-color: var(--reader-background);
  font-family: var(--novel-font-family);
  font-size: var(--novel-font-size);
  line-height: var(--novel-line-height);
  text-align: var(--novel-text-align);
  touch-action: pan-y;
  scrollbar-width: none;
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
.novel-reader.vertical { overflow-y: auto; column-count: auto; }
.novel-reader.vertical-turn { touch-action: pan-x; }
.chapter-text { white-space: pre-wrap; overflow-wrap: anywhere; }
.chapter-markup :deep(img) { max-width: 100%; height: auto; }
.chapter-markup :deep(p) { margin: 0 0 1em; }
.chapter-markup.indent-paragraphs :deep(p[data-reader-indent-normalized="true"]) { text-indent: 2em; }
.chapter-title { margin: 0 0 1.2em; font-size: 1.35em; line-height: 1.4; text-align: left; break-inside: avoid; break-after: avoid; }
</style>
