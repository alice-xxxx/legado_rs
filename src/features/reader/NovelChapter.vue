<script setup lang="ts">
import { computed } from "vue";
import { replacementRules } from "../settings/configurationStores";
import { applyTextRules, applyMarkupRules } from "./textReplacements";
const props = defineProps<{ text?: string; markup?: string; bookId: string; title: string; index: number; paragraphIndent?: boolean }>();
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
</script>
<template>
  <article class="chapter-content" :data-reader-chapter="index">
    <h1 v-if="showTitle" class="chapter-title">{{ title }}</h1>
    <div v-if="text != null" class="chapter-text">{{ displayText }}</div>
    <div v-else class="chapter-markup" :class="{ 'indent-paragraphs': paragraphIndent }" v-html="displayMarkup"></div>
  </article>
</template>
<style scoped>
/* Keep horizontal insets inside each column: WebKit excludes the column
   container's trailing padding from its scrollable width on the final page. */
.chapter-content { padding-inline: var(--gutter); }
.chapter-text { white-space: pre-wrap; overflow-wrap: anywhere; }
.chapter-markup :deep(img) { max-width: 100%; height: auto; }
.chapter-markup :deep(p) { margin: 0 0 1em; }
.chapter-markup.indent-paragraphs :deep(p[data-reader-indent-normalized="true"]) { text-indent: 2em; }
.chapter-title { margin: 0 0 1.2em; font-size: 1.35em; line-height: 1.4; text-align: left; break-inside: avoid; break-after: avoid; }
</style>
