<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppBootstrap, AppTask } from "../api/types";
import {
  beginFrontendRestoreTransition,
  clearShelfBatchRecovery,
  lockShelfBatchRecovery,
} from "./recoveryState";

const unlisteners: UnlistenFn[] = [];
let disposed = false;

onMounted(async () => {
  try {
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", () => {
      beginFrontendRestoreTransition();
      clearShelfBatchRecovery();
    }));
    unlisteners.push(await listen<Partial<AppTask>>("task-recovery-required", (event) => {
      const task = event.payload;
      if (task.status !== "recoveryRequired") return;
      const title = task.kind === "refreshChapters" || task.kind === "checkNewChapters"
        ? "目录任务需要恢复"
        : "书架操作需要恢复";
      const message = task.result?.warning || task.error || "请重启应用完成资源恢复。";
      lockShelfBatchRecovery(title, message);
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
