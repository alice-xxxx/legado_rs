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
}>(), {
  title: "",
  backLabel: "返回",
  showBackLabel: false,
});
const emit = defineEmits<{ back: [] }>();
const theme = useTheme();
</script>

<template>
  <KNavbar :key="theme" component="header" class="subpage-header" :title="title" :center-title="false"
    inner-class="mx-auto max-w-[980px] overflow-visible" title-class="min-w-0 flex-1 truncate" :subtitle="subtitle">
    <template v-if="$slots.title" #title><slot name="title" /></template>
    <template #left>
      <BackButton :label="backLabel" :show-label="showBackLabel" :disabled="backDisabled" :data-back-local="localBack || undefined" @click="emit('back')" />
    </template>
    <template v-if="$slots.actions" #right><slot name="actions" /></template>
  </KNavbar>
</template>
