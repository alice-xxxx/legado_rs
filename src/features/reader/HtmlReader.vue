<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import ReaderTextContent from "./ReaderTextContent.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
</script>
<template>
  <ReaderPage v-bind="props">
    <ReaderTextContent :reader="reader" :paragraph-indent="reader.readingBook.value?.original?.toLowerCase().endsWith('.epub') === true" />
  </ReaderPage>
</template>
