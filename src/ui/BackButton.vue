<script setup lang="ts">
import { Button as KButton } from "konsta/vue";
import PrototypeIcon from "./PrototypeIcon.vue";
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
    <PrototypeIcon name="left" />
    <span v-if="showLabel">{{ label }}</span>
  </KButton>
</template>
