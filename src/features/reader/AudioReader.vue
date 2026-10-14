<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { settings } from "../settings/settingsStore";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
const content = computed(() => reader.readingContent.value?.kind === "media" ? reader.readingContent.value : null);
import { NativeReaderAudio, usesNativeAudio } from "./nativeAudio";
const native = usesNativeAudio();
let nativePlayer: NativeReaderAudio | undefined;
watch(content, value => {
  if (!native || !value) return;
  nativePlayer?.dispose();
  const book = reader.readingBook.value;
  nativePlayer = new NativeReaderAudio({ key: `${book?.id}:${reader.readingChapterIndex.value}`,
    title: reader.currentChapterTitle.value || book?.title || "音频",
    artist: book?.author, artworkUrl: book?.coverSrc,
    progressUpdatedAtMs: book?.progress?.updatedAtMs ?? 0 });
  nativePlayer.playbackRate = settings.value.reader.audioPlaybackRate;
  reader.setReaderMediaElement(nativePlayer);
}, { immediate: true });
watch(() => settings.value.reader.audioPlaybackRate, rate => { if (nativePlayer) nativePlayer.playbackRate = rate; });
onBeforeUnmount(() => nativePlayer?.dispose());
</script>
<template>
  <ReaderPage v-bind="props">
    <div class="audio-reader">
      <audio v-if="content && !native" :ref="reader.setReaderMediaElement" :key="content.src"
         :playbackRate.prop="settings.reader.audioPlaybackRate" controls playsinline preload="metadata" />
    </div>
  </ReaderPage>
</template>
<style scoped>
.audio-reader { width: 100%; height: 100%; display: grid; place-items: center; }
audio { display: block; width: 100%; max-height: 100%;  }
</style>
