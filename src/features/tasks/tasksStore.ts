import { computed, ref, shallowRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readTasks } from "../../api/database";
import type { AppBootstrap, AppTask, MutationAck } from "../../api/types";
import { getFrontendRestoreRevision } from "../../app/recoveryState";
import { errorText, notify } from "../../app/notifications";

export const tasks = shallowRef<AppTask[]>([]);
export const tasksLoaded = ref(false);
export const tasksLoading = ref(false);
export const tasksError = ref("");
export const latestTaskUpdate = shallowRef<Partial<AppTask> | null>(null);
const recoveryTaskSignaled = ref(false);
export const taskRecoveryRequired = computed(() => recoveryTaskSignaled.value
  || tasks.value.some(task => task.status === "recoveryRequired"));

const recoveryTasks = new Map<string, AppTask>();
const runningUpdates = new Map<string, Partial<AppTask>>();
const unlisteners: UnlistenFn[] = [];
let requestRevision = 0;
let lifetime = 0;
let pendingRead: Promise<void> | null = null;
let refreshQueued = false;
let initialization: Promise<void> | null = null;

/** SQLite stores lifecycle checkpoints; events supply live progress between commits. */
export function refreshTasks(): Promise<void> {
  if (pendingRead) { refreshQueued = true; return pendingRead; }
  const revision = ++requestRevision;
  const restoreRevision = getFrontendRestoreRevision();
  const session = lifetime;
  const isCurrent = () => revision === requestRevision && session === lifetime
    && restoreRevision === getFrontendRestoreRevision();
  tasksLoading.value = !tasksLoaded.value;
  const read = (async () => {
    try {
      // Resource and task notifications in the same turn share one read.
      await Promise.resolve();
      do {
        refreshQueued = false;
        if (!isCurrent()) return;
        const document = { tasks: await readTasks() };
        if (!isCurrent()) return;
        const byId = new Map(document.tasks.map(task => [task.id, task]));
        for (const [id, update] of runningUpdates) {
          const task=byId.get(id);
          if (task?.status==="running" && (update.updatedAtMs ?? 0)>task.updatedAtMs) byId.set(id,{...task,...update});
          else runningUpdates.delete(id);
        }
        for (const task of recoveryTasks.values()) byId.set(task.id, task);
        tasks.value = [...byId.values()];
        tasksLoaded.value = true;
        tasksError.value = "";
      } while (refreshQueued);
    } catch (error) {
      if (!isCurrent()) return;
      tasksError.value = errorText(error);
      throw error;
    } finally {
      if (isCurrent()) tasksLoading.value = false;
    }
  })();
  pendingRead = read;
  void read.finally(() => { if (pendingRead === read) pendingRead = null; }).catch(() => {});
  return read.then(async () => {
    if (session === lifetime && restoreRevision === getFrontendRestoreRevision()) await ensureTasksLoaded();
  });
}

export async function ensureTasksLoaded(): Promise<void> {
  while (pendingRead) {
    const read = pendingRead;
    await read;
    if (pendingRead === read) break;
  }
  if (!tasksLoaded.value) await refreshTasks();
  if (tasksError.value) throw new Error(tasksError.value);
}

function applyRecoveryTask(summary: Partial<AppTask>): void {
  if (!summary.id || summary.status !== "recoveryRequired") return;
  recoveryTaskSignaled.value = true;
  const previous = tasks.value.find(task => task.id === summary.id);
  const candidate = {
    ...previous, ...summary,
    page: previous?.page ?? 1, fromIndex: previous?.fromIndex ?? 0,
    status: "recoveryRequired" as const,
    error: summary.error ?? previous?.error,
    result: { ...previous?.result, ...summary.result, recoveryRequired: true },
  };
  if (candidate.kind && candidate.completed !== undefined && candidate.total !== undefined
    && candidate.createdAtMs !== undefined && candidate.updatedAtMs !== undefined) {
    const task = candidate as AppTask;
    recoveryTasks.set(task.id, task);
    tasks.value = previous ? tasks.value.map(entry => entry.id === task.id ? task : entry) : [...tasks.value, task];
  }
  latestTaskUpdate.value = { ...summary };
}

function resetForRestore(): void {
  requestRevision++;
  pendingRead = null;
  refreshQueued = false;
  recoveryTasks.clear();
  runningUpdates.clear();
  recoveryTaskSignaled.value = false;
  tasks.value = [];
  tasksLoaded.value = false;
  tasksError.value = "";
  latestTaskUpdate.value = null;
}

function reportError(error: unknown): void {
  notify(`任务状态读取失败：${errorText(error)}`, "error");
}

/** Installed once at startup, independently of route and reader lifetimes. */
export function initializeTaskStore(): Promise<void> {
  if (initialization) return initialization;
  const session = lifetime;
  async function retain<T>(event: string, handler: (payload: T) => void): Promise<void> {
    const unlisten = await listen<T>(event, ({ payload }) => { if (session === lifetime) handler(payload); });
    if (session !== lifetime) unlisten();
    else unlisteners.push(unlisten);
  }
  initialization = (async () => {
    await retain<{ kind: string; resource: MutationAck }>("resource-updated", payload => {
      if (payload.kind === "tasks") void refreshTasks().catch(reportError);
    });
    await retain<{ taskId: string; resource: MutationAck; task?: Partial<AppTask> }>("task-updated", payload => {
      const summary = payload.task;
      if (summary?.status==="running") runningUpdates.set(payload.taskId,summary);
      else runningUpdates.delete(payload.taskId);
      const index = tasks.value.findIndex(task => task.id === payload.taskId);
      const current = tasks.value[index];
      // Progress changes no request fields. Lifecycle transitions reload the list.
      if (!pendingRead && tasksLoaded.value && current?.status === "running"
        && summary?.status === "running" && summary.completed !== undefined
        && summary.total !== undefined && summary.updatedAtMs !== undefined) {
        if (summary.updatedAtMs < current.updatedAtMs || summary.completed < current.completed) return;
        const next = [...tasks.value];
        next[index] = { ...current, completed: summary.completed, total: summary.total, updatedAtMs: summary.updatedAtMs };
        tasks.value = next;
        latestTaskUpdate.value = next[index]!;
        return;
      }
      const restoreRevision = getFrontendRestoreRevision();
      void refreshTasks().then(() => {
        if (session !== lifetime || restoreRevision !== getFrontendRestoreRevision()) return;
        const task = tasks.value.find(entry => entry.id === payload.taskId);
        if (task) latestTaskUpdate.value = { ...task };
      }).catch(reportError);
    });
    await retain<Partial<AppTask>>("task-recovery-required", applyRecoveryTask);
    await retain<AppBootstrap>("app-state-updated", () => {
      resetForRestore();
      void refreshTasks().catch(reportError);
    });
    if (session === lifetime) await ensureTasksLoaded();
  })().catch(error => {
    if (session !== lifetime) return;
    tasksLoading.value = false;
    tasksError.value = errorText(error);
    reportError(error);
  });
  return initialization;
}

export function disposeTaskStore(): void {
  lifetime++;
  requestRevision++;
  for (const unlisten of unlisteners.splice(0)) unlisten();
  initialization = null;
  pendingRead = null;
  refreshQueued = false;
}
