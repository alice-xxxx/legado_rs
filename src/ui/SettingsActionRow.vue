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
    content-class="ps-0!" inner-class="pe-0!"
    title-font-size-ios="text-[length:var(--ui-font-body)]"
    title-font-size-material="text-[length:var(--ui-font-body)]"
    :colors="{ primaryTextIos: 'text-[var(--app-text)]', primaryTextMaterial: 'text-[var(--app-text)]' }"
    @click="$emit('click')">
    <template #media><PrototypeIcon :name="icon" /></template>
  </KListItem>
</template>
