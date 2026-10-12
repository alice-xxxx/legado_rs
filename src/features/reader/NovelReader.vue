<script setup lang="ts">
import { computed } from "vue";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import NovelContent from "./NovelContent.vue";
const content = computed(() => reader.readingContent.value);
</script>
<template>
  <ReaderPage v-bind="props">
    <NovelContent v-if="content?.kind === 'text' || content?.kind === 'richText'"
      :text="content.kind === 'text' ? content.text : undefined" :markup="content.kind === 'richText' ? content.markup : undefined"
      :book-id="reader.readingBook.value!.id" :title="reader.currentChapterTitle.value"
      @ready="reader.setReaderViewport" @layout="reader.onReaderLayout" @text-change="reader.onReaderTextChange" />
  </ReaderPage>
</template>
