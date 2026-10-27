<script setup lang="ts">
import { computed } from "vue";
import type { ShelfResource } from "../../api/types";
import { readingProgressPercent } from "../books/readingProgress";

const props = defineProps<{ book: ShelfResource["books"][number] }>();
const progress = computed(() => readingProgressPercent(props.book.progress, props.book.chapterCount));
</script>

<template>
  <span class="library-book-info">
    <span class="library-book-heading">
      <strong class="library-book-title" :title="book.title">{{ book.title }}</strong>
      <span class="library-book-progress" title="阅读进度">{{ progress }}%</span>
    </span>
    <span v-if="book.latestChapter?.trim()" class="library-book-latest" :title="book.latestChapter">{{ book.latestChapter }}</span>
  </span>
</template>

<style scoped>
@layer components {
.library-book-info {
  display: grid;
  gap: 4px;
  min-width: 0;
  padding-top: 10px;
  text-align: left;
}
.library-book-heading {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.library-book-title {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  min-width: 0;
  overflow: hidden;
  overflow-wrap: anywhere;
  color: var(--app-text);
  font-size: var(--ui-font-body);
  font-weight: 600;
  line-height: 1.45;
}
.library-book-latest {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  line-height: 1.5;
}
.library-book-progress {
  flex: none;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
}
</style>
