<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import PdfReaderPage from "./PdfReaderPage.vue";
</script>
<template>
  <ReaderPage v-bind="props">
    <PdfReaderPage v-if="reader.readingPdfPage.value" :src="reader.readingPdfPage.value.src"
      :page-index="reader.readingPdfPage.value.pageIndex" :default-zoom="reader.activePdfZoom.value"
      :initial-password="reader.readingPdfInitialPassword.value" @initial-password-used="reader.consumeImportedPdfPassword"
      :find-open="reader.readerControlsOpen.value && reader.readerControlsMode.value === 'search'"
      @update:find-open="reader.readerControlsOpen.value = $event; reader.readerControlsMode.value = 'search'"
      @loaded="reader.readerPageCount.value = 1" @page-selected="reader.selectReaderChapter" @close-reader="props.closeReader()" />
  </ReaderPage>
</template>
