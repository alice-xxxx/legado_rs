<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import ReaderTextContent from "./ReaderTextContent.vue";
</script>
<template>
  <ReaderPage v-bind="props">
    <ReaderTextContent :reader="reader" />
  </ReaderPage>
</template>
