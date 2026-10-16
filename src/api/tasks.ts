import { command } from "./ipc";
import { tasks, refreshTasks, ensureTasksLoaded } from "../features/tasks/tasksStore";
import { getFrontendRestoreRevision } from "../app/recoveryState";
import type { TaskResponse, ClearFinishedTasksResponse } from "./types";

export async function taskCommand(name: string, args: Record<string, unknown>): Promise<TaskResponse> {
  const revision = getFrontendRestoreRevision();
  const response = await command<Omit<TaskResponse, "task"> & { taskId: string }>(name, args);
  if (revision !== getFrontendRestoreRevision()) throw new Error("恢复后任务请求已失效。");
  await refreshTasks();
  await ensureTasksLoaded();
  if (revision !== getFrontendRestoreRevision()) throw new Error("恢复后任务请求已失效。");
  const task = tasks.value.find((entry) => entry.id === response.taskId);
  if (!task) throw new Error("任务已不可用，请刷新任务列表。");
  return { ...response, task };
}


export const clearFinishedTasks = () => command<ClearFinishedTasksResponse>("clear_finished_tasks");

export const startBookDownload = (bookId: string) =>
  taskCommand("start_book_download", { bookId });

export const startChapterDownload = (bookId: string, fromIndex: number, count: number) =>
  taskCommand("start_chapter_download", { bookId, fromIndex, count });

export const refreshChapters = (bookId: string) => taskCommand("refresh_chapters", { bookId });

export const checkNewChapters = (bookId: string) => taskCommand("check_new_chapters", { bookId });

export const pauseTask = (taskId: string) => taskCommand("pause_task", { taskId });

export const resumeTask = (taskId: string) => taskCommand("resume_task", { taskId });

export const cancelTask = (taskId: string) => taskCommand("cancel_task", { taskId });

export const retryTask = (taskId: string) => taskCommand("retry_task", { taskId });
