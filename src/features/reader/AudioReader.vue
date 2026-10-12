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
    <div class="audio-reader">
      <audio v-if="content" :ref="reader.setReaderMediaElement" :key="content.src"
         :playbackRate.prop="settings.reader.audioPlaybackRate" controls playsinline preload="metadata" />
    </div>
  </ReaderPage>
</template>
<style scoped>
.audio-reader { width: 100%; height: 100%; display: grid; place-items: center; }
audio { display: block; width: 100%; max-height: 100%;  }
</style>
