<script setup lang="ts">
import { Button as KButton } from "konsta/vue";
withDefaults(defineProps<{
  label: string;
  disabled?: boolean;
  showLabel?: boolean;
}>(), {
  disabled: false,
  showLabel: false,
});

const emit = defineEmits<{ click: [event: MouseEvent] }>();
</script>

<template>
  <KButton inline clear rounded
    type="button"
    class="back-button h-11 min-w-11 gap-1 px-2"
    data-back-action
    :title="label"
    :aria-label="label"
    :disabled="disabled"
    @click="emit('click', $event)"
  >
    <svg class="back-button__icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m14.5 5-7 7 7 7" />
    </svg>
    <span v-if="showLabel">{{ label }}</span>
  </KButton>
</template>

<style scoped>
@layer components {
.back-button__icon {
  width: var(--ui-icon-size);
  height: var(--ui-icon-size);
  flex: none;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}
}
</style>
