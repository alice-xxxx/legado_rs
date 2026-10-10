import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import type { AppTask, BatchRemoveBooksResponse, ResourceDescriptor, ShelfResource, TaskResponse } from "../../api/types";

type ShelfBook = ShelfResource["books"][number];
type ShelfBatchRemovalTarget = Pick<ShelfBook, "id" | "title" | "author">;

export interface ShelfBatchTaskResult {
  operation: "catalog" | "download";
  started: Array<{ id: string; title: string; taskId: string }>;
  skippedLocal: Array<{ id: string; title: string }>;
  skippedActive: Array<{ id: string; title: string }>;
  failed: { id: string; title: string; outcome: "notStarted" | "unknown"; message: string } | null;
  notStarted: Array<{ id: string; title: string }>;
  taskListWarnings: Array<{ id: string; title: string; message: string }>;
  blockedByRecovery: boolean;
}

interface ShelfBatchActionsOptions {
  shelf: Ref<ShelfResource>;
  visibleBooks: ComputedRef<ShelfResource["books"]>;
  groups: ComputedRef<string[]>;
  tasks: Ref<AppTask[]>;
  recoveryRequired: Ref<boolean>;
  recoveryTitle: Ref<string>;
  recoveryMessage: Ref<string>;
  isShelfScreen: () => boolean;
  getRestoreRevision: () => number;
  refreshShelf: (descriptor: ResourceDescriptor) => Promise<void>;
  refreshTasks: (descriptor?: ResourceDescriptor) => Promise<void>;
  refreshChapters: (bookId: string) => Promise<TaskResponse>;
  startBookDownload: (bookId: string) => Promise<TaskResponse>;
  batchRemoveBooks: (bookIds: string[]) => Promise<BatchRemoveBooksResponse>;
  batchSetBookGroups: (bookIds: string[], groups: string[]) => Promise<ResourceDescriptor>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/**
 * 管理书架多选、批量任务、批量分组和批量移除。
 * 任务结果不确定或后端要求恢复时会设置应用共用的恢复锁，阻止重复提交。
 */
export function useShelfBatchActions(options: ShelfBatchActionsOptions) {
  const {
    shelf,
    visibleBooks,
    groups,
    tasks,
    recoveryRequired,
    recoveryTitle,
    recoveryMessage,
    isShelfScreen,
    getRestoreRevision,
    refreshShelf,
    refreshTasks,
    refreshChapters,
    startBookDownload,
    batchRemoveBooks,
    batchSetBookGroups,
    notify,
    errorText,
  } = options;

  const selectionMode = ref(false);
  const selectedBookIds = ref<string[]>([]);
  const batchBusy = ref(false);
  const batchError = ref("");
  const taskResult = ref<ShelfBatchTaskResult | null>(null);
  const groupsOpen = ref(false);
  const groupBookIds = ref<string[]>([]);
  const groupDraft = ref<string[]>([]);
  const clearGroups = ref(false);
  const newGroup = ref("");
  const removeConfirmOpen = ref(false);
  const removalTargets = ref<ShelfBatchRemovalTarget[]>([]);
  const removalResult = ref<BatchRemoveBooksResponse | null>(null);
  const removalError = ref("");
  const shelfReadError = ref("");
  const removalResultOpen = ref(false);

  const selectedVisibleBookIds = computed(() => {
    const selected = new Set(selectedBookIds.value);
    return visibleBooks.value.filter((book) => selected.has(book.id)).map((book) => book.id);
  });
  const selectedVisibleBooks = computed(() => {
    const selected = new Set(selectedVisibleBookIds.value);
    return visibleBooks.value.filter((book) => selected.has(book.id));
  });
  const selectedLocalBooks = computed(() => selectedVisibleBooks.value.filter((book) => book.id.startsWith("local-")));
  const allVisibleBooksSelected = computed(() =>
    visibleBooks.value.length > 0 && visibleBooks.value.every((book) => selectedBookIds.value.includes(book.id)));

  // 分组或筛选变化后只保留当前可见书籍；批量请求期间保留原选择快照。
  watch(visibleBooks, (books) => {
    if (!selectionMode.value || batchBusy.value) return;
    const visibleIds = new Set(books.map((book) => book.id));
    selectedBookIds.value = selectedBookIds.value.filter((id) => visibleIds.has(id));
  }, { flush: "sync" });

  function enterSelectionMode(): void {
    selectionMode.value = true;
    selectedBookIds.value = [];
    batchError.value = "";
  }

  function exitSelectionMode(): void {
    if (batchBusy.value) return;
    selectionMode.value = false;
    selectedBookIds.value = [];
    groupsOpen.value = false;
    groupBookIds.value = [];
    groupDraft.value = [];
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
  }

  function toggleBookSelection(bookId: string): void {
    if (!selectionMode.value || batchBusy.value || !visibleBooks.value.some((book) => book.id === bookId)) return;
    selectedBookIds.value = selectedBookIds.value.includes(bookId)
      ? selectedBookIds.value.filter((id) => id !== bookId)
      : [...selectedBookIds.value, bookId];
    batchError.value = "";
  }

  function selectVisibleBooks(): void {
    if (batchBusy.value) return;
    selectedBookIds.value = [...new Set([...selectedBookIds.value, ...visibleBooks.value.map((book) => book.id)])];
    batchError.value = "";
  }

  function clearVisibleBookSelection(): void {
    if (batchBusy.value) return;
    const visibleIds = new Set(visibleBooks.value.map((book) => book.id));
    selectedBookIds.value = selectedBookIds.value.filter((id) => !visibleIds.has(id));
    batchError.value = "";
  }

  async function startSelectedTasks(operation: "catalog" | "download"): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const isCurrent = () => restoreRevision === getRestoreRevision();
    const selected = selectedVisibleBooks.value.map((book) => ({ id: book.id, title: book.title }));
    if (!selectionMode.value || !selected.length || batchBusy.value) return;
    if (selected.length > 256) {
      batchError.value = "一次最多批量处理 256 本书，请减少选择。";
      return;
    }

    const skippedLocal = selected.filter((book) => book.id.startsWith("local-"));
    const targets = selected.filter((book) => !book.id.startsWith("local-"));
    const taskKind = operation === "catalog" ? "refreshChapters" : "chapterDownload";
    const taskLabel = operation === "catalog" ? "目录更新" : "章节下载";
    const activeTaskBookIds = new Set(tasks.value.filter((task) => task.kind === taskKind
      && ["queued", "running", "pausing", "paused", "cancelling", "recoveryRequired"].includes(task.status))
      .map((task) => task.bookId)
      .filter((bookId): bookId is string => Boolean(bookId)));
    const skippedActive = targets.filter((book) => activeTaskBookIds.has(book.id));
    const booksToStart = targets.filter((book) => !activeTaskBookIds.has(book.id));
    const started: Array<{ id: string; title: string; taskId: string }> = [];
    const notStarted: Array<{ id: string; title: string }> = [];
    const taskListWarnings: Array<{ id: string; title: string; message: string }> = [];
    let failed: ShelfBatchTaskResult["failed"] = null;
    let blockedByRecovery = recoveryRequired.value;

    if (blockedByRecovery) {
      notStarted.push(...booksToStart);
    } else if (targets.length) {
      batchBusy.value = true;
      taskResult.value = null;
      batchError.value = "";
      try {
        for (let index = 0; index < booksToStart.length; index += 1) {
          const book = booksToStart[index];
          if (!isCurrent()) return;
          if (recoveryRequired.value) {
            blockedByRecovery = true;
            notStarted.push(...booksToStart.slice(index));
            break;
          }

          const activeTask = tasks.value.find((task) => task.bookId === book.id
            && task.kind === taskKind
            && ["queued", "running", "pausing", "paused", "cancelling", "recoveryRequired"].includes(task.status));
          if (activeTask) {
            skippedActive.push(book);
            continue;
          }

          const knownTaskIds = new Set(tasks.value.map((task) => task.id));
          const requestStartedAtMs = Date.now();
          try {
            const response = operation === "catalog"
              ? await refreshChapters(book.id)
              : await startBookDownload(book.id);
            if (!isCurrent()) return;
            const task = response.task;
            if (task.kind !== taskKind || task.bookId !== book.id) {
              throw new Error(`返回的任务与所选书籍不匹配，无法确认${taskLabel}状态。`);
            }

            started.push({ id: book.id, title: book.title, taskId: task.id });
            tasks.value = [...tasks.value.filter((entry) => entry.id !== task.id), task];
            try {
              await refreshTasks(response.resource);
            } catch (error) {
              taskListWarnings.push({ id: book.id, title: book.title, message: errorText(error) });
            }
          } catch (error) {
            if (!isCurrent()) return;
            let taskListRefreshed = false;
            let reconciledTask: AppTask | undefined;
            try {
              await refreshTasks();
              if (!isCurrent()) return;
              taskListRefreshed = true;
              reconciledTask = tasks.value.find((task) => !knownTaskIds.has(task.id)
                && task.bookId === book.id
                && task.kind === taskKind
                && task.createdAtMs >= requestStartedAtMs - 1000);
            } catch {
              // 任务列表未能确认请求是否已经到达 Rust。
            }

            if (reconciledTask) {
              started.push({ id: book.id, title: book.title, taskId: reconciledTask.id });
              taskListWarnings.push({
                id: book.id,
                title: book.title,
                message: `命令没有返回成功，但任务列表确认任务已创建：${errorText(error)}`,
              });
            } else {
              failed = {
                id: book.id,
                title: book.title,
                outcome: taskListRefreshed ? "notStarted" : "unknown",
                message: taskListRefreshed
                  ? errorText(error)
                  : `无法确认是否已创建任务；请先确认任务状态，避免重复提交。${errorText(error)}`,
              };
            }
            notStarted.push(...booksToStart.slice(index + 1));
            break;
          }
        }
      } finally {
        batchBusy.value = false;
      }
    }

    if (!isCurrent()) return;
    if (recoveryRequired.value) blockedByRecovery = true;
    taskResult.value = {
      operation,
      started,
      skippedLocal,
      skippedActive,
      failed,
      notStarted,
      taskListWarnings,
      blockedByRecovery,
    };

    if (blockedByRecovery) {
      notify(`恢复尚未完成：已启动 ${started.length} 本，后续 ${notStarted.length} 本未启动。请重启应用后再继续。`, "error");
    } else if (failed?.outcome === "unknown") {
      notify(`${taskLabel}任务提交状态未确认。已启动 ${started.length} 本，后续 ${notStarted.length} 本未启动；请先确认任务状态，避免重复提交。`, "error");
    } else if (failed) {
      notify(`批量${taskLabel}已停止：已启动 ${started.length} 本，当前书籍未启动，后续 ${notStarted.length} 本未启动。`, "error");
    } else if (!started.length) {
      notify(`没有新建任务：本地书跳过 ${skippedLocal.length} 本，已有任务跳过 ${skippedActive.length} 本。`, skippedActive.length ? "error" : "success");
    } else if (taskListWarnings.length) {
      notify(`已为 ${started.length} 本书创建${taskLabel}任务，但任务列表刷新有提示；请查看每本书的处理结果。`, "error");
    } else {
      notify(`已为 ${started.length} 本书创建独立的${taskLabel}任务；本地书跳过 ${skippedLocal.length} 本，已有任务跳过 ${skippedActive.length} 本。`);
    }
  }

  function openRemovalConfirm(): void {
    const targets = selectedVisibleBooks.value.map((book) => ({ id: book.id, title: book.title, author: book.author }));
    if (!targets.length || targets.length > 256 || batchBusy.value || recoveryRequired.value) return;
    removalTargets.value = targets;
    removalResult.value = null;
    removalError.value = "";
    shelfReadError.value = "";
    removalResultOpen.value = false;
    removeConfirmOpen.value = true;
  }

  function cancelRemoval(): void {
    if (batchBusy.value) return;
    removeConfirmOpen.value = false;
    removalTargets.value = [];
  }

  function removalTargetTitle(bookId: string): string {
    const target = removalTargets.value.find((entry) => entry.id === bookId);
    if (!target) return bookId;
    return target.author ? `${target.title} · ${target.author}` : target.title;
  }

  function closeRemovalResult(): void {
    removalResultOpen.value = false;
  }

  function dismissRemovalSummary(): void {
    if (recoveryRequired.value) return;
    removalResult.value = null;
    removalTargets.value = [];
    removalError.value = "";
    shelfReadError.value = "";
  }

  async function submitRemoval(): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const isCurrent = () => restoreRevision === getRestoreRevision();
    const targets = [...removalTargets.value];
    const bookIds = targets.map((book) => book.id);
    if (!bookIds.length || bookIds.length > 256 || batchBusy.value || recoveryRequired.value) return;
    batchBusy.value = true;
    removalError.value = "";
    shelfReadError.value = "";
    try {
      const result = await batchRemoveBooks(bookIds);
      if (!isCurrent()) return;
      const targetIds = new Set(bookIds);
      const completedIds = [...new Set(result.completedIds.filter((id) => targetIds.has(id)))];
      const completed = new Set(completedIds);
      selectedBookIds.value = selectedBookIds.value.filter((id) => !completed.has(id));
      removalResult.value = result;
      removeConfirmOpen.value = false;
      removalResultOpen.value = true;

      const needsRecovery = result.recoveryRequired
        || result.notAttemptedIds.length > 0
        || result.errors.some((entry) => entry.outcome === "unknown" || entry.commitState === "indeterminate")
        || result.errors.some((entry) => entry.recoveryRequired)
        || result.warnings.some((entry) => entry.recoveryRequired);
      const notCommittedCount = result.errors.filter((entry) => entry.outcome === "notCommitted").length;
      const unknownCount = result.errors.filter((entry) => entry.outcome === "unknown").length;
      if (needsRecovery) {
        recoveryRequired.value = true;
        recoveryTitle.value = "需要重启应用完成恢复";
        recoveryMessage.value = `批量移除已停止。已提交 ${completedIds.length} 本；请重启应用完成恢复后，再查看未处理书籍。`;
      }

      if (completedIds.length) {
        const currentShelf = shelf.value;
        shelf.value = { ...currentShelf, books: currentShelf.books.filter((book) => !completed.has(book.id)) };
        try {
          await refreshShelf(result.shelf);
          if (!isCurrent()) return;
        } catch (error) {
          if (!isCurrent()) return;
          shelfReadError.value = `删除结果已确认，但书架刷新失败：${errorText(error)}`;
        }
      }

      if (!isCurrent()) return;
      if (needsRecovery) {
        notify("批量移除需要恢复；请重启应用后查看书架。", "error");
      } else if (result.errors.length || result.warnings.length || result.notAttemptedIds.length) {
        notify(`批量移除已处理：已提交 ${completedIds.length} 本，未提交 ${notCommittedCount} 本，状态未知 ${unknownCount} 本，未执行 ${result.notAttemptedIds.length} 本。`, "error");
      } else {
        notify(`已从书架移除 ${completedIds.length} 本书。`);
      }
    } catch (error) {
      if (!isCurrent()) return;
      removalError.value = `没有收到逐本处理结果，无法确认本次删除状态：${errorText(error)}`;
      removeConfirmOpen.value = false;
      removalResultOpen.value = true;
      recoveryRequired.value = true;
      recoveryTitle.value = "批量删除结果未确认";
      recoveryMessage.value = "没有收到逐本处理结果，无法确认本批状态。请重启应用后检查书架；本会话已暂停书架批量修改，避免重复提交。";
      notify("没有收到逐本处理结果，请重启应用检查书架后再继续。", "error");
    } finally {
      batchBusy.value = false;
    }
  }

  function openGroups(): void {
    const bookIds = [...selectedVisibleBookIds.value];
    if (!bookIds.length || bookIds.length > 256 || batchBusy.value || recoveryRequired.value) return;
    groupBookIds.value = bookIds;
    const selectedBooks = shelf.value.books.filter((book) => bookIds.includes(book.id));
    groupDraft.value = groups.value.filter((group) => selectedBooks.every((book) => (book.groups ?? []).includes(group)));
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
    groupsOpen.value = true;
  }

  function toggleGroup(group: string): void {
    if (batchBusy.value || recoveryRequired.value || clearGroups.value) return;
    groupDraft.value = groupDraft.value.includes(group)
      ? groupDraft.value.filter((entry) => entry !== group)
      : [...groupDraft.value, group];
    batchError.value = "";
  }

  function addGroup(): void {
    if (batchBusy.value || recoveryRequired.value) return;
    const group = newGroup.value.trim();
    if (!group) return;
    if (!groupDraft.value.includes(group)) groupDraft.value = [...groupDraft.value, group];
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
  }

  function setClearGroups(clear: boolean): void {
    if (batchBusy.value || recoveryRequired.value) return;
    clearGroups.value = clear;
    if (clear) groupDraft.value = [];
    batchError.value = "";
  }

  function closeGroups(): void {
    if (batchBusy.value) return;
    groupsOpen.value = false;
    groupBookIds.value = [];
    groupDraft.value = [];
    clearGroups.value = false;
    newGroup.value = "";
  }

  async function submitGroups(): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const isCurrent = () => restoreRevision === getRestoreRevision();
    const bookIds = [...groupBookIds.value];
    if (!bookIds.length || batchBusy.value || recoveryRequired.value || (!groupDraft.value.length && !clearGroups.value)) return;
    const nextGroups = clearGroups.value ? [] : [...groupDraft.value];
    batchBusy.value = true;
    batchError.value = "";
    try {
      const descriptor = await batchSetBookGroups(bookIds, nextGroups);
      if (!isCurrent()) return;
      selectedBookIds.value = selectedBookIds.value.filter((id) => !bookIds.includes(id));
      groupsOpen.value = false;
      groupBookIds.value = [];
      groupDraft.value = [];
      clearGroups.value = false;
      newGroup.value = "";
      try {
        await refreshShelf(descriptor);
        if (!isCurrent()) return;
        notify(`已为 ${bookIds.length} 本书替换分组。`);
      } catch (error) {
        if (!isCurrent()) return;
        batchError.value = `分组已保存，但重新读取书架失败：${errorText(error)}`;
        notify(batchError.value, "error");
      }
    } catch (error) {
      if (!isCurrent()) return;
      batchError.value = `批量设置分组失败：${errorText(error)}`;
      notify(batchError.value, "error");
    } finally {
      batchBusy.value = false;
      if (isCurrent() && !isShelfScreen()) exitSelectionMode();
    }
  }

  return {
    selectionMode,
    selectedBookIds,
    selectedVisibleBookIds,
    selectedVisibleBooks,
    selectedLocalBooks,
    allVisibleBooksSelected,
    batchBusy,
    batchError,
    taskResult,
    groupsOpen,
    groupBookIds,
    groupDraft,
    clearGroups,
    newGroup,
    removeConfirmOpen,
    removalTargets,
    removalResult,
    removalError,
    shelfReadError,
    removalResultOpen,
    enterSelectionMode,
    exitSelectionMode,
    toggleBookSelection,
    selectVisibleBooks,
    clearVisibleBookSelection,
    startSelectedTasks,
    openRemovalConfirm,
    cancelRemoval,
    removalTargetTitle,
    closeRemovalResult,
    dismissRemovalSummary,
    submitRemoval,
    openGroups,
    toggleGroup,
    addGroup,
    setClearGroups,
    closeGroups,
    submitGroups,
  };
}
