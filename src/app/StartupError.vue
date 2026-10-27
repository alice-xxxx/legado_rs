<script setup lang="ts">
import { Button as KButton } from "konsta/vue";
import PrototypeIcon from "../ui/PrototypeIcon.vue";
defineProps<{ message: string }>();
const retry = () => window.location.reload();
</script>

<template>
  <main class="page-empty-state min-h-dvh safe-area-top safe-area-bottom" role="alert">
    <span class="empty-state-glyph"><PrototypeIcon name="refresh" /></span>
    <h1 class="ui-page-title">应用暂时无法启动</h1>
    <p>{{ message }}</p>
    <KButton rounded inline @click="retry">重新尝试</KButton>
  </main>
</template>
