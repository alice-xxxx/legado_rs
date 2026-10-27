<script setup lang="ts">
import { Searchbar as KSearchbar, Button as KButton } from "konsta/vue";
import PrototypeIcon from "./PrototypeIcon.vue";

defineProps<{
  query: string;
  searchPlaceholder: string;
  filterExpanded: boolean;
  actionsExpanded: boolean;
}>();

const emit = defineEmits<{
  "update:query": [query: string];
  "toggle-filter": [event: MouseEvent];
  "toggle-actions": [event: MouseEvent];
}>();
</script>

<template>
  <header class="managed-list-toolbar">
    <div class="min-w-0">
      <label class="sr-only" for="managed-list-query">{{ searchPlaceholder }}</label>
      <KSearchbar input-id="managed-list-query" class="m-0! w-full! p-0!" :value="query" :placeholder="searchPlaceholder"
        :disable-button="false" @input="emit('update:query', ($event.target as HTMLInputElement).value)" @clear="emit('update:query', '')" />
    </div>
    <div class="managed-list-toolbar__control">
      <KButton rounded inline clear
        type="button"
        class="managed-list-toolbar__button gap-2 size-11 p-0"
        aria-label="筛选列表"
        :aria-expanded="filterExpanded"
        @click="emit('toggle-filter', $event)"
      ><PrototypeIcon name="filter" /></KButton>
      <slot name="filter-menu" />
    </div>
    <div class="managed-list-toolbar__control">
      <KButton rounded inline clear
        type="button"
        class="managed-list-toolbar__button gap-2 size-11 p-0"
        aria-label="更多列表操作"
        :aria-expanded="actionsExpanded"
        @click="emit('toggle-actions', $event)"
      ><PrototypeIcon name="more" /></KButton>
      <slot name="actions-menu" />
    </div>
  </header>
</template>

<style scoped>
@layer components {
.managed-list-toolbar {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 44px 44px;
  align-items: center;
  gap: 5px;
  margin: 0 0 10px;
}

.managed-list-toolbar__control {
  position: relative;
  display: grid;
  place-items: center;
  min-width: 0;
}

.managed-list-toolbar__button {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  padding: 0;
}

.managed-list-toolbar__button :deep(svg) {
  width: 19px;
  height: 19px;
}

@media (max-width: 520px) {
  .managed-list-toolbar {
    grid-template-columns: minmax(0, 1fr) 44px 44px;
    gap: 4px;
  }
}
@media (pointer: coarse) {
  .managed-list-toolbar { grid-template-columns: minmax(0, 1fr) 44px 44px; }
  .managed-list-toolbar__button { width: 44px; height: 44px; }
}
}
</style>
