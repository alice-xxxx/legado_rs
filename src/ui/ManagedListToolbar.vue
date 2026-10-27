<script setup lang="ts">
import { Button as KButton } from "konsta/vue";
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
    <label class="managed-list-toolbar__search">
      <PrototypeIcon name="search" />
      <input
        :value="query"
        type="search"
        autocomplete="off"
        :placeholder="searchPlaceholder"
        :aria-label="searchPlaceholder"
        @input="emit('update:query', ($event.target as HTMLInputElement).value)"
      />
    </label>
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

.managed-list-toolbar__search {
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  height: var(--ui-control-height);
  padding: 0 10px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
  background: var(--app-surface);
  color: var(--app-muted);
}

.managed-list-toolbar__search :deep(svg) {
  width: 18px;
  height: 18px;
  flex: none;
}

.managed-list-toolbar__search input {
  width: 100%;
  min-width: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--app-text);
  font: inherit;
  font-size: var(--ui-font-body);
}

.managed-list-toolbar__search input::placeholder {
  color: var(--app-muted);
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
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--app-text);
  cursor: pointer;
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

  .managed-list-toolbar__search {
    height: var(--ui-control-height);
  }
}
@media (pointer: coarse) {
  .managed-list-toolbar { grid-template-columns: minmax(0, 1fr) 44px 44px; }
  .managed-list-toolbar__button { width: 44px; height: 44px; }
  .managed-list-toolbar__search { height: 44px; }
}
}
</style>
