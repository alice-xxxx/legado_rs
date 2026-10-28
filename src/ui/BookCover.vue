<script setup lang="ts">
import { ref } from "vue";
import PrototypeIcon from "./PrototypeIcon.vue";

defineProps<{ src?: string | null; alt?: string }>();
const failedSource = ref<string | null>();
</script>

<template>
  <span class="book-cover-art">
    <img v-if="src && src !== failedSource" :src="src" :alt="alt ?? ''" loading="lazy" decoding="async" @error="failedSource = src" />
    <PrototypeIcon v-else name="book" />
  </span>
</template>

<style scoped>
@layer components {
.book-cover-art { display: grid; place-items: center; width: 100%; height: 100%; overflow: hidden; color: var(--app-muted); background: var(--app-panel-softer); }
.book-cover-art img { display: block; width: 100%; height: 100%; object-fit: contain; -webkit-touch-callout: none; -webkit-user-drag: none; user-select: none; }
.book-cover-art > .prototype-icon { width: 40%; max-width: 40px; height: auto; }
}
</style>
