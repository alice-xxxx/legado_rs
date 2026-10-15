<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import type { ImagePage } from "../../api/types";
defineProps<{ pages: ImagePage[]; scale: "fit-width" | "actual-size"; mode: "vertical" | "paged" }>();
const emit = defineEmits<{
  ready: [element: HTMLElement | null];
  layout: [];
  scroll: [];
  touchstart: [event: TouchEvent];
  touchend: [event: TouchEvent];
  tap: [x: number];
}>();
const viewport = ref<HTMLElement | null>(null);
let lastTouchAt = 0;
onMounted(() => { emit("ready", viewport.value); emit("layout"); });
onBeforeUnmount(() => emit("ready", null));

function onTouchEnd(event: TouchEvent): void {
  lastTouchAt = Date.now();
  emit("touchend", event);
}

function onClick(event: MouseEvent): void {
  event.stopPropagation();
  if (Date.now() - lastTouchAt < 500) return;
  emit("tap", event.clientX);
}
</script>
<template>
  <div ref="viewport" class="comic-reader" :class="{ original: scale === 'actual-size', paged: mode === 'paged' }"
    @load.capture="emit('layout')" @scroll.passive="emit('scroll')"
    @touchstart.passive="emit('touchstart', $event)" @touchend.passive="onTouchEnd"
    @click="onClick">
    <img v-for="page in pages" :key="page.id" :data-page-id="page.id" :src="page.src" alt="" />
  </div>
</template>
<style scoped>
.comic-reader { width: 100%; height: 100%; overflow: auto; touch-action: pan-y; background: var(--app-background); }
.comic-reader :deep(img) { display: block; flex: none; width: 100%; height: auto; scroll-snap-align: start; }
.comic-reader.original :deep(img) { width: auto; max-width: none; }
.comic-reader.paged { display: flex; flex-flow: row nowrap; overflow-x: auto; overflow-y: hidden; direction: ltr; scroll-snap-type: x mandatory; touch-action: pan-x; scrollbar-width: none; }
.comic-reader.paged::-webkit-scrollbar { display: none; }
.comic-reader.paged :deep(img),
.comic-reader.paged.original :deep(img) { flex: 0 0 100%; width: 100%; height: 100%; max-width: 100%; object-fit: contain; }
.comic-reader :deep(p) { margin: 0; }
</style>
