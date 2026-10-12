<script setup lang="ts">
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";

defineProps<{ title: string }>();

function returnToAdvancedSettings(): void {
  void router.push({ name: routeNames.settingsOther });
}
</script>

<template>
  <header class="settings-detail-topbar safe-area-top">
    <BackButton label="返回高级设置" @click="returnToAdvancedSettings" />
    <strong>{{ title }}</strong>
    <span></span>
  </header>
</template>

<style scoped>
.settings-detail-topbar {
  position: sticky;
  top: 0;
  z-index: 5;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 12px;
  min-height: var(--ui-header-height);
  padding-inline: var(--ui-header-gutter);
  border-bottom: 1px solid var(--app-line);
  background: var(--app-header);
  backdrop-filter: blur(14px);
}
.settings-detail-topbar > strong { font-size: var(--ui-font-heading); }
</style>
