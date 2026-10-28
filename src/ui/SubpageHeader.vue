<script setup lang="ts">
import { Navbar as KNavbar, useTheme } from "konsta/vue";
import BackButton from "./BackButton.vue";

withDefaults(defineProps<{
  title?: string;
  backLabel?: string;
  showBackLabel?: boolean;
  backDisabled?: boolean;
  localBack?: boolean;
  subtitle?: string;
  back?: boolean;
  large?: boolean;
  wide?: boolean;
}>(), {
  title: "",
  backLabel: "返回",
  showBackLabel: false,
  back: true,
});
const emit = defineEmits<{ back: [] }>();
const theme = useTheme();
</script>

<template>
  <KNavbar :key="theme" component="header" class="subpage-header" :title="title" :center-title="false" :large="large"
    :inner-class="`mx-auto ${wide ? '' : 'max-w-[var(--app-content-width)]'} overflow-visible! px-[var(--ui-page-gutter)]!`"
    left-class="shrink-0" right-class="shrink-0 gap-1 px-1 [&_.k-button]:h-11 [&_.k-button]:min-w-11"
    :title-class="$slots.title ? 'min-w-0 flex-1' : 'min-w-0 flex-1 truncate'" :subtitle="subtitle">
    <template v-if="$slots.title" #title><slot name="title" /></template>
    <template v-if="back" #left>
      <BackButton :label="backLabel" :show-label="showBackLabel" :disabled="backDisabled" :data-back-local="localBack || undefined" @click="emit('back')" />
    </template>
    <template v-if="$slots.actions" #right><slot name="actions" /></template>
  </KNavbar>
</template>

<style scoped>
@layer components {
.subpage-header { flex: none; }
.subpage-header :deep(> .px-safe-4) { width: 100%; max-width: var(--app-content-width); margin-inline: auto; padding-inline: var(--ui-page-gutter); }
}
</style>
