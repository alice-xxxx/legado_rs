import { ref, shallowRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readTasks } from "../../api/database";
import type { AppTask } from "../../api/types";
import { errorText, notify } from "../../app/notifications";

// 仅保留旧记录的读取和清理，不参与正文请求或下载。
export const tasks = shallowRef<AppTask[]>([]);
export const tasksLoaded = ref(false);
export const tasksLoading = ref(false);
export const tasksError = ref("");
const unlisteners: UnlistenFn[] = [];
let generation = 0;
let pending: Promise<void> | null = null;
let initialization: Promise<void> | null = null;

export function refreshTasks(): Promise<void> {
  if (pending) return pending;
  const current = generation;
  tasksLoading.value = true;
  const request = readTasks().then(records => {
    if (current !== generation) return;
    tasks.value = records;
    tasksLoaded.value = true;
    tasksError.value = "";
  }).catch(error => {
    if (current === generation) tasksError.value = errorText(error);
    throw error;
  }).finally(() => {
    if (current === generation) tasksLoading.value = false;
    if (pending === request) pending = null;
  });
  pending = request;
  return request;
}

export const ensureTasksLoaded = () => tasksLoaded.value ? Promise.resolve() : refreshTasks();

export function initializeTaskStore(): Promise<void> {
  if (initialization) return initialization;
  const report = (error: unknown) => notify(`历史记录读取失败：${errorText(error)}`, "error");
  initialization = (async () => {
    unlisteners.push(await listen<{ kind: string }>("resource-updated", ({ payload }) => {
      if (payload.kind === "tasks") void refreshTasks().catch(report);
    }));
    unlisteners.push(await listen("app-state-updated", () => {
      generation++;
      pending = null;
      tasksLoaded.value = false;
      void refreshTasks().catch(report);
    }));
    await ensureTasksLoaded();
  })().catch(report);
  return initialization;
}

export function disposeTaskStore(): void {
  generation++;
  for (const unlisten of unlisteners.splice(0)) unlisten();
  initialization = null;
  pending = null;
}
