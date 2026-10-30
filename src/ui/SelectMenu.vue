<script setup lang="ts" generic="T extends string | number">
import { Button as KButton, List as KList, ListItem as KListItem } from "konsta/vue";
import { computed, nextTick, ref, useId, watch } from "vue";
import KPopover from "./DropdownPopover.vue";
import PrototypeIcon from "./PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "./useOutsidePointerDismiss";

const props = defineProps<{
  modelValue: T;
  options: readonly { value: NoInfer<T>; label: string; disabled?: boolean; group?: string }[];
  label: string;
  id?: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: T]; open: [] }>();
const generatedId = useId();
const triggerId = computed(() => props.id ?? generatedId);
const selected = computed(() => props.options.find(option => option.value === props.modelValue));
const opened = ref(false);
const trigger = ref<InstanceType<typeof KButton>>();
const menuElement = () => document.getElementById(`${generatedId}-menu`);
function close(restoreFocus = false): void {
  opened.value = false;
  if (restoreFocus) trigger.value?.$el.focus({ preventScroll: true });
}
async function open(): Promise<void> {
  if (props.disabled || opened.value) return;
  emit("open");
  opened.value = true;
  await nextTick();
  const root = menuElement();
  const current = root?.querySelector<HTMLElement>('[aria-checked="true"]:not(:disabled)')
    ?? root?.querySelector<HTMLElement>('button:not(:disabled)');
  current?.focus({ preventScroll: true });
  if (root && current) root.scrollTop = Math.max(0, current.offsetTop - (root.clientHeight - current.offsetHeight) / 2);
}
function choose(option: (typeof props.options)[number]): void {
  if (option.disabled || props.disabled) return;
  emit("update:modelValue", option.value);
  close(true);
}
function navigate(event: KeyboardEvent): void {
  if (event.key === "Tab") { close(true); return; }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const items = [...(menuElement()?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
  const index = items.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
    : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
  items[next]?.focus();
}
useOutsidePointerDismiss(() => opened.value,
  target => target instanceof Node && Boolean(trigger.value?.$el.contains(target) || menuElement()?.contains(target)),
  () => close());
watch(() => props.disabled, disabled => { if (disabled) close(); });
</script>

<template>
  <div class="select-menu">
    <KButton ref="trigger" :id="triggerId" clear rounded type="button" class="min-h-11 min-w-11 max-w-full justify-end! gap-2 ps-2! pe-0!"
      :disabled="disabled" :aria-label="`${label}：${selected?.label ?? ''}`" aria-haspopup="menu"
      :aria-expanded="opened" :aria-controls="opened ? `${generatedId}-menu` : undefined"
      @click="opened ? close() : open()" @keydown.down.prevent="open" @keydown.up.prevent="open">
      <span class="truncate">{{ selected?.label ?? label }}</span><PrototypeIcon name="chevron-right" class="size-4 rotate-90 shrink-0" />
    </KButton>
      <KPopover v-if="opened" opened :target="trigger?.$el" :backdrop="false" class="select-menu-popover">
        <KList nested menu-list :id="`${generatedId}-menu`" role="menu" :aria-label="label"
          class="max-h-[min(65dvh,var(--dropdown-height))] overflow-y-auto overscroll-contain" @keydown="navigate">
          <template v-for="(option, index) in options" :key="option.value">
            <KListItem title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]" v-if="option.group && option.group !== options[index - 1]?.group" group-title :title="option.group" />
            <KListItem title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]" menu-list-item link-component="button"
              :menu-list-item-active="option.value === modelValue" :title="option.label" content-class="w-[calc(100%-1rem)] material:w-[calc(100%-2rem)] text-start"
              :link-props="{ type: 'button', role: 'menuitemradio', 'aria-checked': option.value === modelValue, disabled: option.disabled, class: option.disabled ? 'opacity-40' : '' }"
              @click="choose(option)" />
          </template>
        </KList>
      </KPopover>
  </div>
</template>

<style scoped>
@layer components {
.select-menu { display: flex; justify-content: flex-end; min-width: 0; max-width: 100%; }
}
</style>
