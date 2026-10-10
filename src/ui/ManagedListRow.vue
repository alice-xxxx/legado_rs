<script setup lang="ts">
import PrototypeIcon from "./PrototypeIcon.vue";

withDefaults(defineProps<{
  name: string;
  subtitle: string;
  enabled?: boolean;
  active?: boolean;
  busy?: boolean;
  batchMode?: boolean;
  selected?: boolean;
  menuOpen?: boolean;
  showMenu?: boolean;
}>(), {
  enabled: undefined,
  active: false,
  busy: false,
  batchMode: false,
  selected: false,
  menuOpen: false,
  showMenu: true,
});

const emit = defineEmits<{
  open: [];
  "toggle-menu": [];
  select: [selected: boolean];
}>();
</script>

<template>
  <article class="managed-list-row" :class="{ 'is-active': active, 'is-batch': batchMode, 'has-menu': showMenu }">
    <input
      v-if="batchMode"
      class="managed-list-row__checkbox"
      type="checkbox"
      :checked="selected"
      :disabled="busy"
      @click.stop
      @change="emit('select', ($event.target as HTMLInputElement).checked)"
    />
    <button type="button" class="managed-list-row__main" :disabled="busy" @click="emit('open')">
      <span class="managed-list-row__status" :class="{ 'is-disabled': enabled !== true }"></span>
      <span class="managed-list-row__copy"><strong>{{ name }}</strong><small>{{ subtitle }}</small></span>
    </button>
    <div v-if="showMenu" class="managed-list-row__menu-wrap">
      <slot name="menu-trigger">
        <button
          type="button"
          class="managed-list-row__menu-button"
          :class="{ 'is-open': menuOpen }"
          :disabled="busy"
          @click.stop="emit('toggle-menu')"
        ><PrototypeIcon name="more" /></button>
      </slot>
      <div v-if="menuOpen" class="managed-list-row__menu" @click.stop>
        <slot name="menu" />
      </div>
    </div>
  </article>
</template>

<style scoped>
.managed-list-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 30px;
  align-items: center;
  gap: 3px;
  min-width: 0;
  min-height: 54px;
  padding: 0 3px;
  border-radius: 10px;
}

.managed-list-row:not(.has-menu) {
  grid-template-columns: minmax(0, 1fr);
}

.managed-list-row:hover,
.managed-list-row.is-active {
  background: var(--app-accent-soft);
}

.managed-list-row.is-batch {
  grid-template-columns: 24px minmax(0, 1fr) 30px;
}

.managed-list-row.is-batch:not(.has-menu) {
  grid-template-columns: 24px minmax(0, 1fr);
}

.managed-list-row__checkbox {
  justify-self: center;
  width: 14px;
  height: 14px;
  margin: 0;
  accent-color: var(--app-accent);
}

.managed-list-row__main {
  display: grid;
  grid-template-columns: 9px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
  width: 100%;
  min-width: 0;
  min-height: 54px;
  padding: 7px 8px 7px 9px;
  border: 0;
  background: transparent;
  color: var(--app-text);
  text-align: left;
  cursor: pointer;
}

.managed-list-row__main:disabled {
  cursor: default;
  opacity: .6;
}

.managed-list-row__status {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--app-success);
}

.managed-list-row__status.is-disabled {
  background: #c7cad0;
}

.managed-list-row__copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
}

.managed-list-row__copy strong,
.managed-list-row__copy small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.managed-list-row__copy strong {
  font-size: 11px;
  font-weight: 650;
}

.managed-list-row__copy small {
  color: var(--app-muted);
  font-size: 9px;
}

.managed-list-row__menu-wrap {
  position: relative;
  display: grid;
  place-items: center;
}

.managed-list-row__menu-button {
  display: grid;
  place-items: center;
  width: 29px;
  height: 32px;
  padding: 0;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: #858997;
  cursor: pointer;
}

.managed-list-row__menu-button:hover,
.managed-list-row__menu-button.is-open {
  background: var(--app-accent-soft);
  color: var(--app-accent-ink);
}

.managed-list-row__menu-button:disabled {
  cursor: default;
  opacity: .45;
}

.managed-list-row__menu-button :deep(svg) {
  width: 18px;
  height: 18px;
}

.managed-list-row__menu {
  position: absolute;
  top: calc(100% + 3px);
  right: 0;
  z-index: 60;
  display: grid;
  gap: 2px;
  min-width: 148px;
  max-width: min(220px, calc(100vw - 16px));
  padding: 6px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
  background: #fff;
  box-shadow: 0 12px 30px #22223824;
}

.managed-list-row__menu :slotted(button) {
  min-height: 34px;
  padding: 0 9px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--app-text);
  text-align: left;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}

.managed-list-row__menu :slotted(button:hover:not(:disabled)) {
  background: var(--app-panel-softer);
}

.managed-list-row__menu :slotted(button.is-danger) {
  color: var(--app-danger);
}

.managed-list-row__menu :slotted(button:disabled) {
  cursor: default;
  opacity: .5;
}
</style>
