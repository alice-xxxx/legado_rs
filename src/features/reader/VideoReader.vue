<script setup lang="ts">
import { computed } from "vue";
import { settings } from "../settings/settingsStore";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
const content = computed(() => reader.readingContent.value?.kind === "media" ? reader.readingContent.value : null);
</script>
<template>
  <ReaderPage v-bind="props">
    <div class="video-reader">
      <video v-if="content" :ref="reader.setReaderMediaElement" :key="content.src"
         :playbackRate.prop="settings.reader.videoPlaybackRate" controls playsinline preload="metadata" />
    </div>
  </ReaderPage>
</template>
<style scoped>
.video-reader { width: 100%; height: 100%; display: flex; align-items: center; background: #000; }
video { display: block; width: 100%; height: 100%; background: #000; object-fit: contain; }
</style>
