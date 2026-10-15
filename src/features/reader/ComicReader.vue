<script setup lang="ts">
import { computed } from "vue";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import ComicContent from "./ComicContent.vue";
const content = computed(() => reader.readingContent.value?.kind === "images" ? reader.readingContent.value : null);
</script>
<template>
  <ReaderPage v-bind="props" v-slot="{ comicReadMode, comicCounter, comicTouchStart, comicTouchEnd, comicTapAt }">
    <ComicContent v-if="content" :pages="content.pages" :scale="reader.comicScaleMode.value" :mode="comicReadMode"
      @ready="reader.setReaderViewport($event); comicCounter()" @layout="reader.applyComicScrollPosition(); comicCounter()"
      @scroll="comicCounter" @touchstart="comicTouchStart" @touchend="comicTouchEnd" @tap="comicTapAt" />
  </ReaderPage>
</template>
