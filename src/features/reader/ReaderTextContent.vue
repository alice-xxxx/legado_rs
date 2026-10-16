<script setup lang="ts">
import NovelContent from "./NovelContent.vue";
import type { ReaderContextFeatures } from "./readerContext";
defineProps<{ reader: ReaderContextFeatures; paragraphIndent?: boolean }>();
</script>
<template>
  <NovelContent v-if="reader.readingContent.value?.kind === 'text' || reader.readingContent.value?.kind === 'richText'"
    :text="reader.readingContent.value.kind === 'text' ? reader.readingContent.value.text : undefined"
    :markup="reader.readingContent.value.kind === 'richText' ? reader.readingContent.value.markup : undefined"
    :book-id="reader.readingBook.value!.id" :title="reader.currentChapterTitle.value" :chapter-index="reader.readingChapterIndex.value"
    :paragraph-indent="paragraphIndent" :chapters="reader.continuousChapters.value"
    @ready="reader.setReaderViewport" @layout="reader.onReaderLayout" @text-change="reader.onReaderTextChange" />
  <NovelContent v-for="chapter in reader.previewChapters.value" :key="chapter.index" class="reader-chapter-preview" inert aria-hidden="true"
    :text="chapter.content.kind === 'text' ? chapter.content.text : undefined"
    :markup="chapter.content.kind === 'richText' ? chapter.content.markup : undefined"
    :book-id="reader.readingBook.value!.id" :title="chapter.title" :chapter-index="chapter.index" :paragraph-indent="paragraphIndent"
    @ready="reader.setReaderChapterPreview(chapter.index, $event)" />
</template>
