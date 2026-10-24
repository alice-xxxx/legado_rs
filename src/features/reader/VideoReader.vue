<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { settings } from "../settings/settingsStore";
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
const content = computed(() => reader.readingContent.value?.kind === "media" ? reader.readingContent.value : null);
const videoElement = ref<HTMLVideoElement | null>(null);
let fullscreenVideo: HTMLVideoElement | null = null;
let webkitFullscreenVideo: HTMLVideoElement | null = null;
let orientationUpdate = Promise.resolve();

type VideoOrientation = "portrait" | "landscape" | "restore";
type WebkitVideoElement = HTMLVideoElement & { webkitDisplayingFullscreen?: boolean };

function setNativeVideoOrientation(orientation: VideoOrientation): void {
  if (!isTauri()) return;
  orientationUpdate = orientationUpdate.catch(() => {}).then(async () => {
    await invoke<boolean>("set_video_orientation", { orientation });
  }).catch(error => console.warn("Video orientation could not be changed:", error));
}

function orientationForVideo(video: HTMLVideoElement): "portrait" | "landscape" {
  return video.videoWidth > video.videoHeight ? "landscape" : "portrait";
}

function beginVideoFullscreen(video: HTMLVideoElement): void {
  fullscreenVideo = video;
  setNativeVideoOrientation(orientationForVideo(video));
}

function endVideoFullscreen(): void {
  if (!fullscreenVideo) return;
  fullscreenVideo = null;
  webkitFullscreenVideo = null;
  setNativeVideoOrientation("restore");
}

function onWebkitBeginFullscreen(event: Event): void {
  const video = event.currentTarget as HTMLVideoElement;
  webkitFullscreenVideo = video;
  beginVideoFullscreen(video);
}

function onWebkitEndFullscreen(): void {
  webkitFullscreenVideo = null;
  endVideoFullscreen();
}

function onDocumentFullscreenChange(): void {
  const video = videoElement.value;
  if (video && document.fullscreenElement === video) {
    beginVideoFullscreen(video);
    return;
  }
  if (video && (webkitFullscreenVideo === video || (video as WebkitVideoElement).webkitDisplayingFullscreen)) return;
  endVideoFullscreen();
}

function onVideoMetadata(event: Event): void {
  const video = event.currentTarget as HTMLVideoElement;
  if (fullscreenVideo === video) setNativeVideoOrientation(orientationForVideo(video));
}

function setVideoElement(element: unknown): void {
  const next = element instanceof HTMLVideoElement ? element : null;
  const previous = videoElement.value;
  if (previous && previous !== next) {
    previous.removeEventListener("webkitbeginfullscreen", onWebkitBeginFullscreen);
    previous.removeEventListener("webkitendfullscreen", onWebkitEndFullscreen);
    previous.removeEventListener("loadedmetadata", onVideoMetadata);
  }
  videoElement.value = next;
  reader.setReaderMediaElement(element);
  if (next) {
    next.addEventListener("webkitbeginfullscreen", onWebkitBeginFullscreen);
    next.addEventListener("webkitendfullscreen", onWebkitEndFullscreen);
    next.addEventListener("loadedmetadata", onVideoMetadata);
  } else {
    endVideoFullscreen();
  }
}

onMounted(() => document.addEventListener("fullscreenchange", onDocumentFullscreenChange));
onBeforeUnmount(() => {
  document.removeEventListener("fullscreenchange", onDocumentFullscreenChange);
  endVideoFullscreen();
});
</script>
<template>
  <ReaderPage v-bind="props">
    <div class="video-reader">
      <video v-if="content" :ref="setVideoElement" :key="content.src"
         :defaultPlaybackRate.prop="settings.reader.videoPlaybackRate"
         :playbackRate.prop="settings.reader.videoPlaybackRate" controls playsinline preload="metadata" />
    </div>
  </ReaderPage>
</template>
<style scoped>
.video-reader { width: 100%; height: 100%; display: flex; align-items: center; background: #000; }
video { display: block; width: 100%; height: 100%; background: #000; object-fit: contain; }
</style>
