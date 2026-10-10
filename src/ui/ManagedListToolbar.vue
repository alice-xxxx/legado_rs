<script setup lang="ts">
import PrototypeIcon from "./PrototypeIcon.vue";

withDefaults(defineProps<{
  query: string;
  searchPlaceholder: string;
  filterActive?: boolean;
  filterExpanded: boolean;
  actionsExpanded: boolean;
}>(), {
  filterActive: false,
});

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
        @input="emit('update:query', ($event.target as HTMLInputElement).value)"
      />
    </label>
    <div class="managed-list-toolbar__control">
      <button
        type="button"
        class="managed-list-toolbar__button"
        :class="{ 'is-active': filterActive, 'is-open': filterExpanded }"
        @click="emit('toggle-filter', $event)"
      ><PrototypeIcon name="filter" /></button>
      <slot name="filter-menu" />
    </div>
    <div class="managed-list-toolbar__control">
      <button
        type="button"
        class="managed-list-toolbar__button"
        :class="{ 'is-open': actionsExpanded }"
        @click="emit('toggle-actions', $event)"
      ><PrototypeIcon name="more" /></button>
      <slot name="actions-menu" />
    </div>
  </header>
</template>

<style scoped>
.managed-list-toolbar {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 32px 32px;
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
  height: 38px;
  padding: 0 10px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
  background: #fff;
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
  font-size: 11px;
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
  color: #343640;
  cursor: pointer;
}

.managed-list-toolbar__button:hover,
.managed-list-toolbar__button.is-active,
.managed-list-toolbar__button.is-open {
  background: var(--app-accent-soft);
  color: var(--app-accent-ink);
}

.managed-list-toolbar__button :deep(svg) {
  width: 19px;
  height: 19px;
}

@media (max-width: 520px) {
  .managed-list-toolbar {
    grid-template-columns: minmax(0, 1fr) 30px 30px;
    gap: 4px;
  }

  .managed-list-toolbar__search {
    height: 36px;
  }
}
</style>
