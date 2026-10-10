import { command } from "./ipc";
import type { ResourceDescriptor, AppTask, TaskResponse, ClearFinishedTasksResponse } from "./types";

export const tasksResource = () => command<{ resource: ResourceDescriptor }>("tasks_resource");

export const clearFinishedTasks = () => command<ClearFinishedTasksResponse>("clear_finished_tasks");

export const startBookDownload = (bookId: string) =>
  command<TaskResponse>("start_book_download", { bookId });

export const startChapterDownload = (bookId: string, fromIndex: number, count: number) =>
  command<TaskResponse>("start_chapter_download", { bookId, fromIndex, count });

export const refreshChapters = (bookId: string) => command<TaskResponse>("refresh_chapters", { bookId });

export const checkNewChapters = (bookId: string) => command<TaskResponse>("check_new_chapters", { bookId });

export const pauseTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("pause_task", { taskId });

export const resumeTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("resume_task", { taskId });

export const cancelTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("cancel_task", { taskId });

export const retryTask = (taskId: string) => command<TaskResponse>("retry_task", { taskId });
