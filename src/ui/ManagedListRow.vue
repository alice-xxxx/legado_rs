<script setup lang="ts">
import { Checkbox as KCheckbox, Button as KButton, Popover as KPopover } from "konsta/vue";
import { ref } from "vue";
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
const menuButton = ref<InstanceType<typeof KButton>>();
</script>

<template>
  <article class="managed-list-row" :class="{ 'is-batch': batchMode, 'has-menu': showMenu, 'is-selected': selected }">
    <KCheckbox v-if="batchMode"
      class="managed-list-row__checkbox"
      :aria-label="`选择${name}`"
      :checked="selected"
      :disabled="busy"
      @click.stop
      @change="emit('select', ($event.target as HTMLInputElement).checked)"
     ><span class="sr-only">选择{{ name }}</span></KCheckbox>
    <KButton inline clear type="button" class="managed-list-row__main grid! h-auto! justify-start! rounded-xl! px-0! text-start! text-current" :disabled="busy" @click="emit('open')">
      <span class="managed-list-row__status" :class="{ 'is-disabled': enabled !== true }"></span>
      <span class="managed-list-row__copy"><strong>{{ name }}</strong><small>{{ subtitle }}</small></span>
    </KButton>
    <div v-if="showMenu" class="managed-list-row__menu-wrap">
      <slot name="menu-trigger">
        <KButton ref="menuButton" rounded inline clear
          type="button"
          class="managed-list-row__menu-button size-11 p-0 text-current"
          :aria-label="`${name}的更多操作`"
          :aria-expanded="menuOpen"
          :disabled="busy"
          @click.stop="emit('toggle-menu')"
        ><PrototypeIcon name="more" /></KButton>
      </slot>
      <KPopover v-if="menuOpen" opened :backdrop="false" :target="menuButton?.$el" class="managed-list-row__menu w-56 max-w-[calc(100vw-32px)]" @click.stop>
        <div class="grid gap-1 p-2"><slot name="menu" /></div>
      </KPopover>
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

.managed-list-row.is-selected { background: var(--app-accent-soft); }

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
  width: 24px;
  height: 24px;
  margin: 0;
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

.managed-list-row__copy strong, .managed-list-row__copy small {
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
  width: 44px;
  height: 44px;
  padding: 0;
}

.managed-list-row__menu-button :deep(svg) {
  width: 18px;
  height: 18px;
}
@media (pointer: coarse) {
  .managed-list-row.has-menu { grid-template-columns: minmax(0, 1fr) 44px; }
  .managed-list-row.is-batch.has-menu { grid-template-columns: 24px minmax(0, 1fr) 44px; }
}
}
</style>
