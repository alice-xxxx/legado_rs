<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import NovelContent from "./NovelContent.vue";
</script>
<template>
  <ReaderPage v-bind="props">
    <NovelContent v-if="reader.readingContent.value?.kind === 'text' || reader.readingContent.value?.kind === 'richText'"
      :text="reader.readingContent.value.kind === 'text' ? reader.readingContent.value.text : undefined" :markup="reader.readingContent.value.kind === 'richText' ? reader.readingContent.value.markup : undefined"
      :book-id="reader.readingBook.value!.id" :title="reader.currentChapterTitle.value"
      @ready="reader.setReaderViewport" @layout="reader.onReaderLayout" @text-change="reader.onReaderTextChange" />
  </ReaderPage>
</template>
