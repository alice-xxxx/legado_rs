<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import NovelContent from "./NovelContent.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
</script>
<template>
  <ReaderPage v-bind="props">
    <NovelContent v-if="reader.readingContent.value?.kind === 'richText'"
      :markup="reader.readingContent.value.markup"
      :book-id="reader.readingBook.value!.id" :title="reader.currentChapterTitle.value"
      @ready="reader.setReaderViewport" @layout="reader.onReaderLayout" @text-change="reader.onReaderTextChange" />
  </ReaderPage>
</template>
