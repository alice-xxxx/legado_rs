<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppBootstrap } from "../api/types";
import {
  beginFrontendRestoreTransition,
  clearShelfBatchRecovery,
} from "./recoveryState";

const unlisteners: UnlistenFn[] = [];
let disposed = false;

onMounted(async () => {
  try {
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", () => {
      beginFrontendRestoreTransition();
      clearShelfBatchRecovery();
    }));

  } catch (error) {
    if (!disposed) console.error("Recovery event listener registration failed", error);
  }
});

onBeforeUnmount(() => {
  disposed = true;
  for (const unlisten of unlisteners) unlisten();
  unlisteners.length = 0;
});
</script>

<template><span hidden /></template>
