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
    :title="label"
    :disabled="disabled"
    @click="emit('click', $event)"
  >
    <svg class="back-button__icon" viewBox="0 0 24 24">
      <path d="m14.5 5-7 7 7 7" />
    </svg>
    <span v-if="showLabel">{{ label }}</span>
  </button>
</template>

<style scoped>
.back-button {
  display: inline-flex;
  width: 44px;
  height: 44px;
  min-width: 44px;
  min-height: 44px;
  flex: none;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0;
  border: 0;
  border-radius: 12px;
  color: var(--app-muted);
  background: var(--app-panel-soft);
  font: inherit;
  cursor: pointer;
}

.back-button__icon {
  width: 20px;
  height: 20px;
  flex: none;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.back-button:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.back-button:disabled {
  opacity: .45;
  cursor: default;
}
</style>
