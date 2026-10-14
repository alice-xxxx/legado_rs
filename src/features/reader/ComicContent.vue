<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import type { ImagePage } from "../../api/types";
defineProps<{ pages: ImagePage[]; scale: "fit-width" | "actual-size" }>();
const emit = defineEmits<{ ready: [element: HTMLElement | null]; layout: []; scroll: []; touchstart: [event: TouchEvent]; touchend: [event: TouchEvent] }>();
const viewport = ref<HTMLElement | null>(null);
onMounted(() => { emit("ready", viewport.value); emit("layout"); });
onBeforeUnmount(() => emit("ready", null));
</script>
<template>
  <div ref="viewport" class="comic-reader" :class="{ original: scale === 'actual-size' }"
    @load.capture="emit('layout')" @scroll.passive="emit('scroll')"
    @touchstart.passive="emit('touchstart', $event)" @touchend.passive="emit('touchend', $event)"
    >
    <img v-for="page in pages" :key="page.id" :data-page-id="page.id" :src="page.src" alt="" />
  </div>
</template>
<style scoped>
.comic-reader { width: 100%; height: 100%; overflow: auto; touch-action: pan-y; background: var(--app-background); }
.comic-reader :deep(img) { display: block; width: 100%; height: auto; scroll-snap-align: start; }
.comic-reader.original :deep(img) { width: auto; max-width: none; }
.comic-reader :deep(p) { margin: 0; }
</style>
