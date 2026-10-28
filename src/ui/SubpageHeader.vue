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
  <KNavbar :key="theme" component="header" class="subpage-header" :title="title" :center-title="$slots.title || $slots.actions ? false : undefined"
    font-size-ios="text-[length:var(--ui-font-body)]" font-size-material="text-[length:var(--ui-font-body)]"
    title-font-size-ios="text-[length:var(--ui-font-heading)]" title-font-size-material="text-[length:var(--ui-font-heading)]"
    subtitle-class="text-[length:var(--ui-font-caption)]"
    :inner-class="`mx-auto ${wide ? '' : 'max-w-[var(--app-content-width)]'} overflow-visible! px-[var(--ui-page-gutter)]!`"
    left-class="shrink-0 material:mx-0 material:me-2" right-class="shrink-0 gap-1 [&_.k-button]:h-11 [&_.k-button]:min-w-11"
    :title-class="`min-w-0 flex-1 first:mx-0! ${$slots.title ? '' : 'truncate'}`" :subtitle="subtitle">
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
}
</style>
