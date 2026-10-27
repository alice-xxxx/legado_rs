<script setup lang="ts" generic="T extends string | number">
import { Button as KButton, Popover as KPopover } from "konsta/vue";
import { computed, onDeactivated, ref, useId } from "vue";
import PrototypeIcon from "./PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "./useOutsidePointerDismiss";

const props = defineProps<{
  modelValue: T;
  options: readonly { value: NoInfer<T>; label: string; disabled?: boolean }[];
  label: string;
  id?: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: T] }>();
const generatedId = useId();
const controlId = computed(() => props.id ?? generatedId);
const selectedLabel = computed(() => props.options.find(option => option.value === props.modelValue)?.label ?? "请选择");
const root = ref<HTMLElement | null>(null);
const opened = ref(false);
useOutsidePointerDismiss(
  () => opened.value,
  target => target instanceof Node && !!root.value?.contains(target),
  () => { opened.value = false; },
);
onDeactivated(() => { opened.value = false; });
function choose(value: T): void {
  emit("update:modelValue", value);
  opened.value = false;
  root.value?.querySelector("button")?.focus();
}
</script>

<template>
  <div ref="root" class="select-menu flex min-w-0 justify-end">
    <KButton :id="controlId" rounded inline clear class="h-11 max-w-full gap-1 px-3 text-current" type="button"
      :disabled="disabled" :aria-label="label" :aria-expanded="opened" :aria-controls="`${controlId}-options`"
      :title="selectedLabel" @click="opened = !opened">
      <span class="min-w-0 truncate">{{ selectedLabel }}</span><PrototypeIcon name="left" class="size-4 shrink-0 -rotate-90" />
    </KButton>
    <Teleport to="body">
    <KPopover v-if="opened" opened :backdrop="false" :target="`#${controlId}`" class="z-[160] w-56 max-w-[calc(100vw-32px)]">
      <div :id="`${controlId}-options`" class="grid max-h-[60dvh] gap-1 overflow-y-auto p-2" role="group" :aria-label="label" @pointerdown.stop>
        <KButton v-for="option in options" :key="option.value" rounded :tonal="modelValue === option.value"
          :clear="modelValue !== option.value" class="h-auto min-h-11 justify-start py-2 text-start whitespace-normal"
          type="button" :disabled="disabled || option.disabled" :aria-pressed="modelValue === option.value"
          @click="choose(option.value)">{{ option.label }}</KButton>
      </div>
    </KPopover>
    </Teleport>
  </div>
</template>
