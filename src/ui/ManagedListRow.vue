<script setup lang="ts">
import { Checkbox as KCheckbox, Button as KButton, Glass as KGlass } from "konsta/vue";
import PrototypeIcon from "./PrototypeIcon.vue";

withDefaults(defineProps<{
  name: string;
  subtitle: string;
  enabled?: boolean;
  busy?: boolean;
  batchMode?: boolean;
  selected?: boolean;
  menuOpen?: boolean;
  showMenu?: boolean;
}>(), {
  enabled: undefined,
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
  <article class="managed-list-row" :class="{ 'is-batch': batchMode, 'has-menu': showMenu }">
    <KCheckbox v-if="batchMode"
      class="managed-list-row__checkbox"
      :aria-label="`选择${name}`"
      :checked="selected"
      :disabled="busy"
      @click.stop
      @change="emit('select', ($event.target as HTMLInputElement).checked)"
     ><span class="sr-only">选择{{ name }}</span></KCheckbox>
    <button type="button" class="managed-list-row__main" :disabled="busy" @click="emit('open')">
      <span class="managed-list-row__status" :class="{ 'is-disabled': enabled !== true }"></span>
      <span class="managed-list-row__copy"><strong>{{ name }}</strong><small>{{ subtitle }}</small></span>
    </button>
    <div v-if="showMenu" class="managed-list-row__menu-wrap">
      <slot name="menu-trigger">
        <KButton rounded inline clear
          type="button"
          class="managed-list-row__menu-button size-11 p-0 text-current"
          :aria-label="`${name}的更多操作`"
          :aria-expanded="menuOpen"
          :disabled="busy"
          @click.stop="emit('toggle-menu')"
        ><PrototypeIcon name="more" /></KButton>
      </slot>
      <KGlass component="div" :highlight="false" v-if="menuOpen" class="managed-list-row__menu rounded-2xl" @click.stop>
        <slot name="menu" />
      </KGlass>
    </div>
  </article>
</template>

<style scoped>
@layer components {
.managed-list-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 44px;
  align-items: center;
  gap: 3px;
  min-width: 0;
  min-height: var(--ui-row-height);
  padding: 0 3px;
  border-radius: 10px;
}

.managed-list-row:not(.has-menu) {
  grid-template-columns: minmax(0, 1fr);
}

.managed-list-row.is-batch {
  grid-template-columns: 32px minmax(0, 1fr) 44px;
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
  min-height: var(--ui-row-height);
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
  background: var(--app-track);
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
  font-size: var(--ui-font-body);
  font-weight: 650;
}

.managed-list-row__copy small {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
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
  color: var(--app-muted);
  cursor: pointer;
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
  background: var(--app-surface);
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
  font-size: var(--ui-font-body);
  cursor: pointer;
}

.managed-list-row__menu :slotted(button.is-danger) {
  color: var(--app-danger);
}

.managed-list-row__menu :slotted(button:disabled) {
  cursor: default;
  opacity: .5;
}
@media (pointer: coarse) {
  .managed-list-row.has-menu { grid-template-columns: minmax(0, 1fr) 44px; }
  .managed-list-row.is-batch.has-menu { grid-template-columns: 24px minmax(0, 1fr) 44px; }
}
}
</style>
