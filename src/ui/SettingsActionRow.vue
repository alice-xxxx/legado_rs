<script setup lang="ts">
import { ListItem as KListItem } from "konsta/vue";
import PrototypeIcon from "./PrototypeIcon.vue";

defineProps<{
  title: string;
  subtitle?: string;
  icon: InstanceType<typeof PrototypeIcon>["$props"]["name"];
  disabled?: boolean;
}>();
defineEmits<{ click: [] }>();
</script>

<template>
  <KListItem link link-component="button" :title="title" :subtitle="subtitle"
    :link-props="{ type: 'button', class: 'w-full text-start', disabled }"
    title-font-size-ios="text-[length:var(--ui-font-body)]"
    title-font-size-material="text-[length:var(--ui-font-body)]"
    :colors="{ primaryTextIos: 'text-[var(--app-text)]', primaryTextMaterial: 'text-[var(--app-text)]' }"
    @click="$emit('click')">
    <template #media><span class="settings-row-icon"><PrototypeIcon :name="icon" /></span></template>
  </KListItem>
</template>

<style scoped>
@layer components {
.settings-row-icon { display: grid; place-items: center; width: var(--ui-icon-container); height: var(--ui-icon-container); border-radius: 10px; color: var(--app-icon-ink); background: var(--app-icon-background); }
}
</style>
