import { command } from "./ipc";
import { readResource } from "./resources";
import type { ResourceDescriptor, TasksResource, TaskResponse, ClearFinishedTasksResponse } from "./types";

export async function taskCommand(name: string, args: Record<string, unknown>): Promise<TaskResponse> {
  const response = await command<Omit<TaskResponse, "task"> & { taskId: string }>(name, args);
  const document = await readResource<TasksResource>(response.resource);
  const task = document.tasks.find((entry) => entry.id === response.taskId);
  if (!task) throw new Error("任务已不可用，请刷新任务列表。");
  return { ...response, task };
}

export const tasksResource = () => command<{ resource: ResourceDescriptor }>("tasks_resource");

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
