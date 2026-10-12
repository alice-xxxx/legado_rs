<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { settings } from "../settings/settingsStore";
defineProps<{ markup: string; scale: "fit-width" | "actual-size" }>();
const emit = defineEmits<{ ready: [element: HTMLElement | null]; layout: [] }>();
const viewport = ref<HTMLElement | null>(null);
onMounted(() => emit("ready", viewport.value));
onBeforeUnmount(() => emit("ready", null));
</script>
<template>
  <div ref="viewport" class="comic-reader" :class="{ original: scale === 'actual-size' }"
    :style="{ backgroundColor: settings.reader.backgroundColor }" @load.capture="emit('layout')" v-html="markup"></div>
</template>
<style scoped>
.comic-reader { width: 100%; height: 100%; overflow: auto; touch-action: pan-y; }
.comic-reader :deep(img) { display: block; width: 100%; height: auto; scroll-snap-align: start; }
.comic-reader.original :deep(img) { width: auto; max-width: none; }
.comic-reader :deep(p) { margin: 0; }
</style>
