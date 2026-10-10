<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { cancelTask, clearFinishedTasks, pauseTask, resumeTask, retryTask, tasksResource } from "../../api/tasks";
import type { AppBootstrap, AppTask, ResourceDescriptor, ShelfResource, TasksResource } from "../../api/types";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import { backupBusy } from "../settings/backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";

const tasks = ref<AppTask[]>([]);
const shelfBooks = ref<ShelfResource["books"]>([]);
const loading = ref(true);
const dataError = ref("");
const recoveryTaskSignaled = ref(false);
const taskRecoveryRequired = computed(() => recoveryTaskSignaled.value
  || tasks.value.some((task) => task.status === "recoveryRequired"));
const mutationLocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value
  || taskRecoveryRequired.value);
const bookTitles = computed(() => new Map(shelfBooks.value.map((book) => [book.id, book.title])));
const unlisteners: UnlistenFn[] = [];
let isUnmounted = false;
let restoreRevision = 0;
let shelfReadRevision = 0;
let tasksReadRevision = 0;
const recoveryTasks = new Map<string, AppTask>();

const retryingTaskIds = ref<string[]>([]);
const commandingTaskIds = ref<string[]>([]);
const batchBusy = ref(false);
const batchResult = ref("");
const clearBusy = ref(false);
const clearResult = ref("");
const actionError = ref("");

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const revision = ++shelfReadRevision;
  const restoreAtStart = restoreRevision;
  const document = await readResource<ShelfResource>(descriptor);
  if (isUnmounted || revision !== shelfReadRevision || restoreAtStart !== restoreRevision) return;
  shelfBooks.value = document.books ?? [];
}

async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const revision = ++tasksReadRevision;
  const restoreAtStart = restoreRevision;
  const resource = descriptor ?? (await tasksResource()).resource;
  const document = await readResource<TasksResource>(resource);
  if (isUnmounted || revision !== tasksReadRevision || restoreAtStart !== restoreRevision) return;
  const byId = new Map((document.tasks ?? []).map((task) => [task.id, task]));
  for (const task of recoveryTasks.values()) byId.set(task.id, task);
  tasks.value = [...byId.values()];
}

function applyRecoveryTask(summary: Partial<AppTask>): void {
  if (!summary.id || summary.status !== "recoveryRequired") return;
  recoveryTaskSignaled.value = true;
  const previous = tasks.value.find((task) => task.id === summary.id);
  const candidate = {
    ...previous,
    ...summary,
    page: previous?.page ?? 1,
    fromIndex: previous?.fromIndex ?? 0,
    status: "recoveryRequired",
    error: summary.error ?? previous?.error,
    result: { ...previous?.result, ...summary.result, recoveryRequired: true },
  };
  if (!candidate.kind || candidate.completed === undefined || candidate.total === undefined
    || candidate.createdAtMs === undefined || candidate.updatedAtMs === undefined) return;
  const task = candidate as AppTask;
  recoveryTasks.set(task.id, task);
  tasks.value = previous
    ? tasks.value.map((entry) => entry.id === task.id ? task : entry)
    : [...tasks.value, task];
}

async function loadBootstrap(snapshot?: AppBootstrap): Promise<void> {
  const restoreAtStart = restoreRevision;
  loading.value = true;
  dataError.value = "";
  try {
    const current = snapshot ?? await appBootstrap();
    if (isUnmounted || restoreAtStart !== restoreRevision) return;
    await Promise.all([refreshShelf(current.shelf), refreshTasks()]);
  } catch (error) {
    if (!isUnmounted && restoreAtStart === restoreRevision) dataError.value = errorText(error);
  } finally {
    if (!isUnmounted && restoreAtStart === restoreRevision) loading.value = false;
  }
}

async function registerListener<T>(event: string, handler: (payload: T) => void): Promise<void> {
  const unlisten = await listen<T>(event, (eventData) => handler(eventData.payload));
  if (isUnmounted) unlisten();
  else unlisteners.push(unlisten);
}

async function listenForUpdates(): Promise<void> {
  await registerListener<ResourceDescriptor>("shelf-updated", (resource) => {
    void refreshShelf(resource).catch((error) => { dataError.value = errorText(error); });
  });
  await registerListener<{ kind: string; resource: ResourceDescriptor }>("resource-updated", ({ kind, resource }) => {
    if (kind === "tasks") void refreshTasks(resource).catch((error) => { dataError.value = errorText(error); });
  });
  await registerListener<{ taskId: string; resource: ResourceDescriptor }>("task-updated", ({ resource }) => {
    void refreshTasks(resource).catch((error) => { dataError.value = errorText(error); });
  });
  await registerListener<Partial<AppTask>>("task-recovery-required", applyRecoveryTask);
  await registerListener<AppBootstrap>("app-state-updated", (snapshot) => {
    restoreRevision += 1;
    shelfReadRevision += 1;
    tasksReadRevision += 1;
    recoveryTasks.clear();
    tasks.value = [];
    recoveryTaskSignaled.value = false;
    void loadBootstrap(snapshot);
  });
}

onMounted(() => {
  void (async () => {
    try {
      await listenForUpdates();
      await loadBootstrap();
    } catch (error) {
      dataError.value = errorText(error);
      loading.value = false;
    }
  })();
});

onUnmounted(() => {
  isUnmounted = true;
  for (const unlisten of unlisteners) unlisten();
});

function returnToSettings(): void {
  void router.push({ name: routeNames.settings });
}

async function openTaskBook(bookId: string): Promise<void> {
  const requestId = requestBookDetailOpen({ kind: "shelf-book", bookId, initialTab: "catalog" });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

function canRetry(task: AppTask): boolean {
  if (task.status !== "failed" && task.status !== "cancelled") return false;
  if (hasBookDetails(task) && !bookTitle(task)) return false;
  if (task.kind === "chapterDownload") {
    return isDownloadSnapshotValid(task)
      && Boolean(task.bookId)
      && task.completed < task.total;
  }
  if (task.kind === "refreshChapters") return Boolean(task.bookId) && task.checkOnly === false;
  if (task.kind === "checkNewChapters") return Boolean(task.bookId) && task.checkOnly === true;
  if (task.kind === "search") {
    return Boolean(task.keyword?.trim()) && Boolean(task.sourceIds?.length) && task.page > 0;
  }
  if (task.kind === "bookSourceCandidates") {
    return Boolean(task.bookId) && Boolean(task.keyword?.trim()) && Boolean(task.sourceIds?.length) && task.page > 0;
  }
  return false;
}

function isDownloadSnapshotValid(task: AppTask): boolean {
  if (task.kind !== "chapterDownload") return true;
  if (task.downloadSnapshotValid !== undefined) return task.downloadSnapshotValid;
  const chapterIds = task.chapterIds;
  if (!chapterIds) return false;
  return task.total > 0
    && task.completed >= 0
    && task.completed <= task.total
    && chapterIds.length === task.total
    && chapterIds.every((id) => typeof id === "string" && id.length > 0)
    && new Set(chapterIds).size === chapterIds.length;
}

function retryDisabled(task: AppTask): boolean {
  return mutationLocked.value
    || taskManagementBusy.value
    || commandingTaskIds.value.includes(task.id)
    || (task.kind === "search" && searchBusy.value)
    || (task.kind === "bookSourceCandidates" && bookSourceSearchBusy.value);
}

function label(task: AppTask): string {
  if (task.kind === "search") return task.keyword ? `搜索“${task.keyword}”` : "书籍搜索";
  const book = bookTitle(task) ?? "书籍";
  if (task.kind === "bookSourceCandidates") return `为《${book}》寻找其他书源`;
  if (task.kind === "chapterDownload") return `准备《${book}》的章节`;
  if (task.kind === "refreshChapters") return `更新《${book}》的目录`;
  if (task.kind === "checkNewChapters") return `检查《${book}》的新章节`;
  return "阅读任务";
}

function bookTitle(task: AppTask): string | undefined {
  const title = task.bookId ? bookTitles.value.get(task.bookId) : undefined;
  return title?.trim() || undefined;
}

function hasBookDetails(task: AppTask): boolean {
  return Boolean(task.bookId)
    && ["chapterDownload", "refreshChapters", "checkNewChapters"].includes(task.kind);
}

function canResume(task: AppTask): boolean {
  return isDownloadSnapshotValid(task)
    && (!hasBookDetails(task) || Boolean(bookTitle(task)));
}

function retryLabel(task: AppTask): string {
  return task.kind === "chapterDownload" && task.completed > 0 ? "继续剩余章节" : "重试";
}

function statusLabel(task: AppTask): string {
  if (task.status === "recoveryRequired") {
    if (task.result?.publicationFailed) return "任务状态已保存，结果发布失败";
    return task.result?.commitState === "committed"
      ? "目录已提交，待恢复"
      : task.result?.commitState === "notCommitted"
        ? "目录未提交，待恢复"
        : task.result?.commitState === "indeterminate"
          ? "提交状态未知，待恢复"
          : "任务状态未确认，待恢复";
  }
  return ({ queued: "等待中", running: "进行中", pausing: "正在暂停", paused: "已暂停", cancelling: "正在取消", cancelled: "已取消", completed: "已完成", failed: "失败", interrupted: "上次未完成" } as Record<string, string>)[task.status] ?? task.status;
}

function recoveryMessage(task: AppTask): string {
  if (task.result?.publicationFailed) {
    return `任务状态已保存（${task.result.persistedTaskStatus ?? "未知"}），但搜索结果发布失败。请重启应用并重新搜索；不会把这次发布失败误报为任务执行失败。`;
  }
  if (task.result?.commitState === "committed") return "目录更新已生效。请重启应用完成缓存清理和日志恢复。";
  if (task.result?.commitState === "notCommitted") return "目录更新未提交，回滚或其他资源恢复尚未完成。请重启应用后再继续。";
  if (task.result?.commitState === "indeterminate") {
    return "无法确认目录更新是否提交。请勿重试；重启应用完成恢复后再检查目录。";
  }
  return "后台任务意外退出，无法确认任务状态是否已持久化。请重启应用检查后再恢复，避免重复执行。";
}

function progress(task: AppTask): number {
  if ((task.status === "completed" && task.completed >= task.total)
    || (task.status === "recoveryRequired" && task.result?.commitState === "committed")
    || (task.status === "recoveryRequired" && task.result?.publicationFailed
      && task.result.persistedTaskStatus === "completed")) return 100;
  if (task.total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round(task.completed / task.total * 100)));
}

const activeStatuses = new Set(["queued", "running", "pausing", "paused", "cancelling"]);
const commandableStatuses = new Set(["queued", "running", "pausing", "cancelling"]);
const searchBusy = computed(() => tasks.value.some((task) => task.kind === "search" && commandableStatuses.has(task.status)));
const bookSourceSearchBusy = computed(() => tasks.value.some((task) => task.kind === "bookSourceCandidates"
  && commandableStatuses.has(task.status)));
const taskManagementBusy = computed(() => batchBusy.value || clearBusy.value
  || commandingTaskIds.value.length > 0 || retryingTaskIds.value.length > 0);
const batchAction = ref<"pause" | "resume" | null>(null);
const expandedTaskId = ref<string | null>(null);
const pendingClearFinished = ref(false);
type TaskFilter = "all" | "active" | "completed" | "failed";
const filter = ref<TaskFilter>("all");
const filters: Array<{ value: TaskFilter; label: string }> = [
  { value: "all", label: "全部" },
  { value: "active", label: "进行中" },
  { value: "completed", label: "已完成" },
  { value: "failed", label: "失败" },
];
const filteredTasks = computed(() => {
  const items = tasks.value.filter(task => {
    switch (filter.value) {
      case "active": return activeStatuses.has(task.status);
      case "completed": return task.status === "completed";
      case "failed": return ["failed", "recoveryRequired", "interrupted"].includes(task.status);
      default: return true;
    }
  });
  return items.sort((a,b) => b.updatedAtMs - a.updatedAtMs);
});
function filterCount(value: TaskFilter): number {
  return tasks.value.filter(task => {
    switch (value) {
      case "active": return activeStatuses.has(task.status);
      case "completed": return task.status === "completed";
      case "failed": return ["failed","recoveryRequired","interrupted"].includes(task.status);
      default: return true;
    }
  }).length;
}
function taskIcon(task: AppTask): "search" | "download" | "refresh" | "source" {
  if (task.kind === "search") return "search";
  if (task.kind === "bookSourceCandidates") return "source";
  if (task.kind === "chapterDownload") return "download";
  return "refresh";
}

function batchTaskIds(action: "pause" | "resume"): string[] {
  const eligibleStatuses = action === "pause" ? new Set(["queued", "running"]) : new Set(["paused", "interrupted"]);
  return tasks.value
    .filter((task) => eligibleStatuses.has(task.status))
    .filter((task) => action !== "resume" || canResume(task))
    .map((task) => task.id);
}

function clearableTaskCount(): number {
  return tasks.value.filter((task) => ["completed", "failed", "cancelled"].includes(task.status)).length;
}

function taskCommandBusy(taskId: string): boolean {
  return commandingTaskIds.value.includes(taskId);
}

async function command(action: "pause" | "resume" | "cancel", taskId: string): Promise<void> {
  if (mutationLocked.value || taskManagementBusy.value || taskCommandBusy(taskId)) return;
  commandingTaskIds.value = [...commandingTaskIds.value, taskId];
  actionError.value = "";
  try {
    const response = action === "pause"
      ? await pauseTask(taskId)
      : action === "resume"
        ? await resumeTask(taskId)
        : await cancelTask(taskId);
    void refreshTasks(response.resource).catch((error) => { dataError.value = errorText(error); });
  } catch (error) {
    actionError.value = `任务操作失败：${errorText(error)}`;
  } finally {
    commandingTaskIds.value = commandingTaskIds.value.filter((id) => id !== taskId);
  }
}

async function runBatch(action: "pause" | "resume"): Promise<void> {
  if (mutationLocked.value || taskManagementBusy.value) return;
  const taskIds = batchTaskIds(action);
  if (!taskIds.length) return;
  batchAction.value = action;
  batchBusy.value = true;
  batchResult.value = "";
  actionError.value = "";
  let completed = 0;
  const failures: string[] = [];
  let latestResource: ResourceDescriptor | null = null;
  try {
    for (const taskId of taskIds) {
      try {
        const response = action === "pause" ? await pauseTask(taskId) : await resumeTask(taskId);
        latestResource = response.resource;
        completed += 1;
      } catch (error) {
        failures.push(errorText(error));
      }
    }
    if (latestResource) {
      void refreshTasks(latestResource).catch((error) => { dataError.value = errorText(error); });
    }
    batchResult.value = failures.length
      ? `已${action === "pause" ? "暂停" : "继续"} ${completed} 个任务，${failures.length} 个未完成操作。`
      : `已${action === "pause" ? "暂停" : "继续"} ${completed} 个任务。`;
    if (failures.length) actionError.value = failures[0];
  } finally {
    batchBusy.value = false;
  }
}

async function retry(taskId: string): Promise<void> {
  if (mutationLocked.value || taskManagementBusy.value || taskCommandBusy(taskId)) return;
  retryingTaskIds.value = [...retryingTaskIds.value, taskId];
  actionError.value = "";
  try {
    const response = await retryTask(taskId);
    void refreshTasks(response.resource).catch((error) => { dataError.value = errorText(error); });
  } catch (error) {
    actionError.value = `重试任务失败：${errorText(error)}`;
  } finally {
    retryingTaskIds.value = retryingTaskIds.value.filter((id) => id !== taskId);
  }
}

async function clearFinished(): Promise<void> {
  const count = clearableTaskCount();
  if (mutationLocked.value || taskManagementBusy.value || !count) return;
  pendingClearFinished.value = false;
  clearBusy.value = true;
  clearResult.value = "";
  actionError.value = "";
  try {
    const response = await clearFinishedTasks();
    void refreshTasks(response.resource).catch((error) => { dataError.value = errorText(error); });
    clearResult.value = response.removedCount
      ? `已清理 ${response.removedCount} 个结束任务。`
      : response.inUseCount
        ? "当前没有可清理的结束任务，部分任务仍被应用状态引用。"
        : "当前没有可清理的结束任务。";
  } catch (error) {
    actionError.value = `清理任务记录失败：${errorText(error)}`;
  } finally {
    clearBusy.value = false;
  }
}
function taskTimeLabel(timestamp: number): string {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return "时间未知";
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const previous = start - 86_400_000;
  const time = date.toLocaleTimeString("zh-CN", { hour:"2-digit", minute:"2-digit", hour12:false });
  if (date.getTime() >= start && date.getTime() <= now.getTime()) return `今天 ${time}`;
  if (date.getTime() >= previous && date.getTime() < start) return `昨天 ${time}`;
  return date.toLocaleDateString("zh-CN", { month:"numeric", day:"numeric" }) + " " + time;
}
function toggleTaskDetails(taskId: string): void {
  expandedTaskId.value = expandedTaskId.value === taskId ? null : taskId;
}
function canOpenTaskBook(task: AppTask): boolean {
  return Boolean(task.bookId) && Boolean(bookTitle(task));
}
function hasTaskDetails(task: AppTask): boolean {
  return canOpenTaskBook(task) || activeStatuses.has(task.status) ||
    (task.status === "interrupted" && !canResume(task)) || canRetry(task) ||
    Boolean(task.error) || Boolean(task.result?.warning) ||
    task.status === "recoveryRequired" || (hasBookDetails(task) && !bookTitle(task));
}
</script>

<template>
  <div class="tasks-workspace">
    <header class="tasks-topbar">
      <BackButton label="返回我的" @click="returnToSettings" />
      <div class="tasks-title"><strong>下载与任务</strong></div>
      <span></span>
    </header>
    <section class="task-center">
      <div class="task-toolbar">
        <nav class="task-filter-tabs">
          <button v-for="item in filters" :key="item.value" type="button"
:class="{ active: filter === item.value }"
            @click="filter = item.value">{{ item.label }}</button>
        </nav>
        <details v-if="batchTaskIds('pause').length || batchTaskIds('resume').length || clearableTaskCount()"
          class="task-batch-menu">
          <summary title="批量任务操作"><PrototypeIcon name="more"/></summary>
          <div>
            <small>进行中 {{ filterCount('active') }} · 共 {{ tasks.length }} 项</small>
            <button v-if="batchTaskIds('pause').length" type="button"
              :disabled="mutationLocked || taskManagementBusy" @click="runBatch('pause')">
              {{ batchBusy && batchAction === 'pause' ? '正在暂停…' : '暂停全部进行中' }}
            </button>
            <button v-if="batchTaskIds('resume').length" type="button"
              :disabled="mutationLocked || taskManagementBusy" @click="runBatch('resume')">
              {{ batchBusy && batchAction === 'resume' ? '正在继续…' : '继续全部已暂停' }}
            </button>
            <button v-if="clearableTaskCount()" type="button" class="task-menu-danger"
              :disabled="mutationLocked || taskManagementBusy"
              @click="pendingClearFinished = true">清理已结束任务（{{ clearableTaskCount() }}）</button>
          </div>
        </details>
      </div>
      <p v-if="dataError" class="task-batch-result task-action-error">读取任务或书架失败：{{ dataError }}</p>
      <p v-if="batchResult" class="task-batch-result">{{ batchResult }}</p>
      <p v-if="clearResult" class="task-batch-result">{{ clearResult }}</p>
      <p v-if="actionError" class="task-batch-result task-action-error">{{ actionError }}</p>
      <p v-if="taskRecoveryRequired" class="task-locked">检测到尚未完成的任务恢复，暂时无法修改后台任务。请重新启动应用完成恢复。</p>

      <div v-if="loading" class="task-empty">正在读取任务…</div>
      <div v-else-if="filteredTasks.length" class="task-list">
        <article v-for="task in filteredTasks" :key="task.id" class="task-card" :class="`task-${task.status}`"
          >
          <span class="task-type-icon"><PrototypeIcon :name="taskIcon(task)"/></span>
          <div class="task-card-copy">
            <button type="button" class="task-name"
              @click="toggleTaskDetails(task.id)">{{ label(task) }}</button>
            <small class="task-status-line">
              {{ statusLabel(task) }}<template v-if="task.total > 0"> · {{ task.completed }} / {{ task.total }}</template>
            </small>
            <div class="task-track"

>
              <span :class="{ indeterminate: task.status === 'running' && task.total <= 0 }"
                :style="{ width: task.total > 0 ? `${progress(task)}%` : '0%' }"></span>
            </div>
          </div>
          <time class="task-updated" :datetime="new Date(task.updatedAtMs).toISOString()"
            :title="new Date(task.updatedAtMs).toLocaleString('zh-CN')">
            {{ taskTimeLabel(task.updatedAtMs) }}
          </time>
          <div class="task-actions">
            <button v-if="task.status === 'running' || task.status === 'queued'"
              class="task-primary-action" type="button" :disabled="mutationLocked || taskManagementBusy"
@click="command('pause',task.id)">
              <PrototypeIcon name="pause"/>
            </button>
            <button v-else-if="(task.status === 'paused' || task.status === 'interrupted') && canResume(task)"
              class="task-primary-action" type="button" :disabled="mutationLocked || taskManagementBusy"
@click="command('resume',task.id)">
              <PrototypeIcon name="play"/>
            </button>
            <button v-else-if="canRetry(task)" class="task-primary-action" type="button"
              :disabled="retryDisabled(task) || clearBusy"
              @click="retry(task.id)"><PrototypeIcon name="refresh"/></button>
            <button v-else-if="hasTaskDetails(task)" class="task-primary-action" type="button"

              @click="toggleTaskDetails(task.id)"><PrototypeIcon name="more"/></button>
          </div>
          <div v-if="expandedTaskId === task.id" class="task-details">
            <p class="task-detail-meta">创建于 {{ new Date(task.createdAtMs).toLocaleString('zh-CN') }}</p>
            <p v-if="task.status === 'recoveryRequired'" class="task-error">{{ recoveryMessage(task) }}</p>
            <p v-if="hasBookDetails(task) && !bookTitle(task)" class="task-retry-message">
              对应书籍已从内容库移除，无法继续旧任务。
            </p>
            <p v-if="task.kind === 'chapterDownload' && !isDownloadSnapshotValid(task)" class="task-retry-message">
              当前下载任务缺少有效章节快照。请取消并重新发起下载。
            </p>
            <p v-if="task.kind === 'chapterDownload' && task.completed > 0 && ['failed','cancelled'].includes(task.status)"
              class="task-retry-message">已完成的章节会保留，重新执行时继续剩余章节。</p>
            <p v-if="task.result?.warning" class="task-retry-message">{{ task.result.warning }}</p>
            <p v-if="task.error" class="task-error">{{ task.error }}</p>
            <div class="task-detail-actions">
              <button v-if="canOpenTaskBook(task)" type="button"
                @click="task.bookId && openTaskBook(task.bookId)">查看书籍</button>
              <button v-if="activeStatuses.has(task.status) || (task.status === 'interrupted' && !canResume(task))"
                type="button" class="task-detail-danger" :disabled="mutationLocked || taskManagementBusy"
                @click="command('cancel',task.id)">{{ taskCommandBusy(task.id) ? '正在操作…' : '取消任务' }}</button>
              <button v-if="canRetry(task)" type="button" :disabled="retryDisabled(task) || clearBusy"
                @click="retry(task.id)">{{ retryingTaskIds.includes(task.id) ? '正在重试…' : retryLabel(task) }}</button>
            </div>
          </div>
          <p v-if="expandedTaskId !== task.id && task.status === 'recoveryRequired'" class="task-error task-recovery-summary">
            恢复未完成。展开此任务查看详情。</p>
          <p v-else-if="expandedTaskId !== task.id && task.error" class="task-error">{{ task.error }}</p>
        </article>
      </div>
      <div v-else class="task-empty">
        {{ tasks.length ? '当前筛选下没有任务。' : '目前没有下载、更新或搜索任务。' }}
      </div>
    </section>
    <div v-if="pendingClearFinished" class="task-confirm-backdrop"
>
      <div class="task-confirm">
        <strong>清理已结束任务？</strong>
        <p>将清理 {{ clearableTaskCount() }} 项已完成、失败或取消的任务记录，不会删除已有的下载内容。正在进行的任务不会受影响。</p>
        <footer>
          <button type="button" @click="pendingClearFinished = false">取消</button>
          <button type="button" class="task-confirm-danger" :disabled="mutationLocked || taskManagementBusy"
            @click="clearFinished">确认清理</button>
        </footer>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tasks-workspace{width:100%;min-height:100%;color:#282b33}
.tasks-topbar{position:sticky;top:0;z-index:35;box-sizing:border-box;height:64px;display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;padding:0 max(18px,calc((100% - 1180px)/2));border-bottom:1px solid var(--app-line);background:rgb(246 247 249 / 92%);backdrop-filter:blur(18px)}
.tasks-title{min-width:0}
.tasks-title strong{font-size:14px;color:#18191d;font-weight:750;line-height:1.25}
.task-center{box-sizing:border-box;width:min(860px,calc(100% - 36px));margin:0 auto;padding:24px 0 54px}
.task-toolbar{display:flex;align-items:center;justify-content:space-between;gap:8px;margin:0 0 18px}
.task-filter-tabs{display:flex;align-items:center;gap:4px;overflow-x:auto;scrollbar-width:none}
.task-filter-tabs button{flex:none;box-sizing:border-box;height:30px;padding:0 10px;border:0;border-radius:9px;background:transparent;color:#7e838b;font-size:9px;cursor:pointer}
.task-filter-tabs button.active{background:#202126;color:white;font-weight:750}
.task-batch-menu{position:relative;margin-left:auto}
.task-batch-menu summary{display:grid;place-items:center;width:32px;height:32px;border-radius:9px;color:#9296a0;list-style:none;cursor:pointer}
.task-batch-menu summary::-webkit-details-marker{display:none}
.task-batch-menu summary :deep(svg){width:18px;height:18px}
.task-batch-menu summary:hover{background:#ececf2}
.task-batch-menu>div{position:absolute;z-index:40;right:0;top:36px;min-width:185px;display:grid;gap:3px;padding:7px;border:1px solid var(--app-line);border-radius:11px;background:#fff;box-shadow:0 16px 40px #20203322}
.task-batch-menu>div>small{padding:7px 9px;color:#9a9fa7;font-size:9px}
.task-batch-menu button{min-height:33px;padding:6px 9px;border:0;border-radius:7px;text-align:left;background:transparent;color:#6c687f;font-size:11px;cursor:pointer}
.task-batch-menu button:hover{background:#f3f0ff}
.task-batch-menu button.task-menu-danger{color:#b25962}
.task-batch-menu button:disabled{opacity:.5;cursor:default}
.task-batch-result,.task-locked{padding:10px 12px;border-radius:9px;background:#f3f0ff;color:#6b5db4;font-size:11px}
.task-batch-result.task-action-error{background:#fff0f1;color:#ac535c}
.task-locked{background:#fff0f1;color:#ac535c}
.task-list{border-top:0}
.task-card{display:grid;grid-template-columns:38px minmax(0,1fr) 80px 36px;align-items:center;gap:10px;min-height:78px;box-sizing:border-box;border-bottom:1px solid var(--app-line)}
.task-type-icon{display:grid;place-items:center;width:36px;height:36px;border-radius:10px;background:var(--app-accent-soft);color:var(--app-accent)}
.task-type-icon :deep(svg){width:19px;height:19px}
.task-card-copy{display:flex;flex-direction:column;align-items:stretch;justify-content:center;gap:4px;min-width:0}
.task-name{display:block;min-width:0;overflow:hidden;padding:0;border:0;background:none;color:#282b33;font-size:10px;font-weight:750;text-align:left;text-overflow:ellipsis;white-space:nowrap;cursor:pointer}
.task-name:hover{color:var(--app-accent)}
.task-status-line{font-size:8px;color:#999da5;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.task-track{position:relative;height:4px;width:100%;overflow:hidden;border-radius:99px;background:#e7e9ee}
.task-track>span{display:block;height:100%;border-radius:inherit;background:var(--app-accent);transition:width .2s}
.task-track>span.indeterminate{position:absolute;top:0;left:0;width:36%!important;transform:translateX(-110%);animation:task-shimmer 1.4s ease-in-out infinite}
.task-failed .task-track>span,.task-recoveryRequired .task-track>span{background:#b85f69}
@keyframes task-shimmer{to{transform:translateX(305%)}}
.task-updated{font-size:8px;line-height:1.35;color:#999da5;text-align:right;white-space:nowrap}
.task-actions{display:flex;align-items:center;justify-content:center;width:36px}
.task-primary-action{display:grid;place-items:center;width:36px;height:36px;padding:0;border:0;border-radius:10px;background:transparent;color:#777d89;cursor:pointer}
.task-primary-action:hover:not(:disabled){background:#ececf2;color:var(--app-accent)}
.task-primary-action:disabled{opacity:.45;cursor:default}
.task-primary-action :deep(svg){width:18px;height:18px}
.task-details{grid-column:2/-1;display:flex;flex-direction:column;gap:8px;padding:0 0 13px;min-width:0}
.task-detail-meta{margin:0;color:#9a9fa9;font-size:9px}
.task-detail-actions{display:flex;flex-wrap:wrap;gap:7px}
.task-detail-actions button{min-height:31px;padding:5px 10px;border:1px solid #e5e6ec;border-radius:8px;background:#fff;color:#6c648b;font-size:10px;cursor:pointer}
.task-detail-actions button.task-detail-danger{color:#aa4a57;background:#fff7f7}
.task-detail-actions button:disabled{opacity:.5;cursor:default}
.task-retry-message,.task-error{grid-column:2/-1;margin:0;padding:0 0 6px;color:#838895;font-size:10px;line-height:1.5;overflow-wrap:anywhere}
.task-error{color:#ad535d}
.task-recovery-summary{padding:0 0 8px}
.task-empty{display:flex;align-items:center;justify-content:center;min-height:170px;color:#999da6;text-align:center;font-size:11px}
.task-confirm-backdrop{position:fixed;inset:0;z-index:95;display:grid;place-items:center;padding:16px;background:#20223377}
.task-confirm{box-sizing:border-box;width:min(400px,100%);padding:21px;border-radius:13px;background:#fff;box-shadow:0 18px 50px #1112222b}
.task-confirm strong{font-size:14px}
.task-confirm p{margin:12px 0;color:#747a86;font-size:11px;line-height:1.65}
.task-confirm footer{display:flex;justify-content:flex-end;gap:8px}
.task-confirm footer button{min-height:35px;padding:0 13px;border:0;border-radius:9px;background:#eceef2;color:#525966;font-size:11px;cursor:pointer}
.task-confirm footer .task-confirm-danger{background:#b95b68;color:#fff}
@media(max-width:860px){
  .tasks-topbar{padding:0 18px}
  .task-center{width:calc(100% - 36px)}
  .task-card{grid-template-columns:36px minmax(0,1fr) 34px;gap:10px}
  .task-updated{display:none}
  .task-type-icon{width:36px;height:36px}
  .task-actions,.task-primary-action{width:34px}
}
@media(max-width:400px){
  .task-center{width:calc(100% - 28px)}
  .task-card{gap:8px}
}
</style>
