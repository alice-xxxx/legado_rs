<script setup lang="ts">
// 返回按钮只统一箭头和触发方式。
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
  <button
    type="button"
    class="back-button"
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
  </button>
</template>

<style scoped>
.back-button {
  display: inline-flex;
  width: var(--ui-control-height);
  height: var(--ui-control-height);
  min-width: var(--ui-control-height);
  min-height: var(--ui-control-height);
  flex: none;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0;
  border: 0;
  border-radius: var(--ui-radius);
  color: var(--app-muted);
  background: transparent;
  font: inherit;
  cursor: pointer;
}

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

.back-button:disabled {
  opacity: .45;
  cursor: default;
}
</style>
