<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { settings } from "../settings/settingsStore";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
const content = computed(() => reader.readingContent.value?.kind === "media" ? reader.readingContent.value : null);
const audioElement = ref<HTMLAudioElement | null>(null);
watch([content, audioElement], ([value, element]) => {
  if (value && element) reader.setReaderMediaElement(element);
}, { flush: "post" });
onBeforeUnmount(() => { audioElement.value?.pause(); reader.setReaderMediaElement(null); });
</script>
<template>
  <ReaderPage v-bind="props">
    <div class="audio-reader">
      <audio v-show="content" ref="audioElement"
         :defaultPlaybackRate.prop="settings.reader.audioPlaybackRate"
         :playbackRate.prop="settings.reader.audioPlaybackRate" controls playsinline preload="metadata" />
    </div>
  </ReaderPage>
</template>
<style scoped>
@layer components {
.audio-reader { width: 100%; height: 100%; display: grid; place-items: center; }
audio { display: block; width: 100%; max-height: 100%;  }
}
</style>
