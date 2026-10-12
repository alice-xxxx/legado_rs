<script setup lang="ts">
import { computed } from "vue";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import ComicContent from "./ComicContent.vue";
const content = computed(() => reader.readingContent.value?.kind === "richText" ? reader.readingContent.value : null);
</script>
<template>
  <ReaderPage v-bind="props">
    <ComicContent v-if="content" :markup="content.markup" :scale="reader.comicScaleMode.value"
      @ready="reader.setReaderViewport" @layout="reader.applyComicScrollPosition" />
  </ReaderPage>
</template>
