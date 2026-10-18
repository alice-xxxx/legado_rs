import { replaceLibraryContent, libraryType } from "../books/contentLibraryState";
import { computed, ref, shallowRef, type Ref } from "vue";
import { cancelTask } from "../../api/tasks";
import { tasks } from "../tasks/tasksStore";
import { bookSourceCandidates } from "./sourcesStore";
import { shelfBatchRecoveryRequired, shelfBatchRecoveryTitle, shelfBatchRecoveryMessage } from "../../app/recoveryState";
import { changeBookSource, changeChapterSource, searchBookSourceCandidates } from "../../api/search";
import { prepareSearchResultBook, prepareAlternativeBook } from "../../api/books";
import { bookContentType } from "../../api/bookContentType";
import { type AppTask, type BookSourceCandidateBatch, type BookResource, type BookSourceMutationResponse, type SearchBookResult } from "../../api/types";
import type { CachedChapterContent } from "../reader/useReaderChapterLoading";
import { loadBookPreview, clearOpenedBook, switchOpenedBook, openedBook } from "../books/bookDetailState";

interface BookSourceSwitchDependencies {
  /** 应用共享的任务、书架、阅读器和书签状态。 */
  readingBook: Ref<BookResource | null>;
  readerVisible: Ref<boolean>;
  readingChapterIndex: Ref<number>;
  readerPageIndex: Ref<number>;
  readingContent: Ref<CachedChapterContent | null>;
  readerProgressDirty: Ref<boolean>;
  readerControlsOpen: Ref<boolean>;
  saveCurrentProgress: () => Promise<void>;
  loadReaderChapter: (index: number) => Promise<boolean>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  getRestoreRevision: () => number;
}

/** 管理整书换源、单章正文替换和候选任务同步。 */
export function useBookSourceSwitch({
  readingBook,
  readerVisible,
  readingChapterIndex,
  readerPageIndex,
  readingContent,
  readerProgressDirty,
  readerControlsOpen,
  saveCurrentProgress,
  loadReaderChapter,
  notify,
  errorText,
  getRestoreRevision,
}: BookSourceSwitchDependencies) {
  const bookSourceSwitchOpen = ref(false);
  const bookSourceSwitchInline = ref(false);
  const bookSourceSwitchBusy = ref(false);
  const chapterSourceBusyResultId = ref<string | null>(null);
  const chapterSourceIdentityConfirmResultId = ref<string | null>(null);
  const chapterSourceError = ref("");
  const bookSourceSearchBusy = ref(false);
  const candidateRetryBusy = ref(false);
  const bookSourceSwitchConfirmOpen = ref(false);
  const bookSourceSwitchFromReader = ref(false);
  const bookSourceSwitchBookId = ref<string | null>(null);
  const bookSourceSwitchBook = computed(() => !bookSourceSwitchOpen.value
    || openedBook.value?.id === bookSourceSwitchBookId.value ? openedBook.value : null);
  const bookSourceEligibleSources = computed(() => {
    const book = bookSourceSwitchBook.value;
    if (!book || book.canChangeSource !== true) return [];
    const type = bookContentType(book);
    const searchType = type === "html" || type === "pdf" ? "text" : type
      ?? bookSourceCandidates.value.find(source => source.id === book.sourceId)?.searchType ?? "text";
    return bookSourceCandidates.value.filter(source => source.id !== book.sourceId && source.searchType === searchType);
  });
  const bookSourceChapterTargetId = ref<string | null>(null);
  const bookSourceChapterTargetTitle = ref<string | null>(null);
  const bookSourceCandidateTaskId = ref<string | null>(null);
  const searchedCandidates = shallowRef<SearchBookResult[]>([]);
  const bookSourceCandidateResults = computed<SearchBookResult[]>(() => {
    const cached = (bookSourceSwitchBook.value?.alternatives ?? []).map((candidate, index) => ({
      resultId: `alternative-${index}`, alternativeIndex: index,
      sourceId: candidate.sourceId, sourceName: candidate.sourceName,
      title: candidate.title, author: candidate.author, coverSrc: candidate.cover,
      latestChapter: candidate.latest, requiresIdentityConfirmation: !candidate.author,
    }));
    if (!bookSourceSwitchOpen.value) return cached;
    const liveSources = new Set(searchedCandidates.value.map(candidate => candidate.sourceId));
    return [...cached.filter(candidate => !liveSources.has(candidate.sourceId)), ...searchedCandidates.value];
  });
  const bookSourceCandidateErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
  const selectedBookSourceCandidate = ref<SearchBookResult | null>(null);
  const bookSourceIdentityConfirmationNeeded = ref(false);
  const bookSourceSwitchError = ref("");
  const bookSourceChangeError = ref("");
  const bookSourceSearchStatus = ref("");
  const bookSourceSearchKeyword = ref("");
  let finishedStreamTaskId: string | null = null;
  let bookSourceSearchGeneration = 0;
  const currentRestore = (revision: number) => revision === getRestoreRevision();

  const bookSourceCurrentChapter = computed(() => {
    const book = bookSourceSwitchBook.value;
    if (!book) return null;
    const currentBook = openedBook.value?.id === book.id
      ? openedBook.value
      : readingBook.value?.id === book.id ? readingBook.value : book;
    if (bookSourceChapterTargetId.value) {
      return currentBook.chapters.find((chapter) => chapter.id === bookSourceChapterTargetId.value) ?? null;
    }
    const readerBook = readingBook.value;
    if (bookSourceSwitchFromReader.value && readerVisible.value && readerBook?.id === book.id) {
      return readerBook.chapters.find((chapter) => chapter.index === readingChapterIndex.value) ?? null;
    }
    const chapterIndex = book.progress?.chapterIndex;
    if (chapterIndex == null) return null;
    return book.chapters.find((chapter) => chapter.index === chapterIndex) ?? null;
  });
  const bookSourceNeedsIdentityConfirmation = computed(() =>
    bookSourceIdentityConfirmationNeeded.value || selectedBookSourceCandidate.value?.requiresIdentityConfirmation === true);
  async function openBookSourceSwitch(fromReader = false, chapterTarget?: { id: string; title: string }): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const book = fromReader ? readingBook.value : openedBook.value;
    await openBookSourceSwitchFor(book, fromReader, chapterTarget, restoreRevision);
    if (fromReader && currentRestore(restoreRevision) && bookSourceSwitchOpen.value && !bookSourceCandidateResults.value.length && !bookSourceSearchBusy.value) {
      await startBookSourceCandidateSearch();
    }
  }

  async function openBookSourceSwitchForBook(book: BookResource, chapterTarget?: { id: string; title: string }, inline = false): Promise<void> {
    await openBookSourceSwitchFor(book, false, chapterTarget, getRestoreRevision(), inline);
  }

  async function openBookSourceSwitchFor(
    book: BookResource | null,
    fromReader: boolean,
    chapterTarget: { id: string; title: string } | undefined,
    restoreRevision: number,
    inline = false,
  ): Promise<void> {
    if (!book || book.canChangeSource !== true || bookSourceSwitchBusy.value || candidateRetryBusy.value) return;
    if (bookSourceSwitchOpen.value && bookSourceSwitchBookId.value === book.id
      && bookSourceSwitchFromReader.value === fromReader && bookSourceChapterTargetId.value === (chapterTarget?.id ?? null)) {
      bookSourceSwitchInline.value = inline;
      return;
    }
    if (fromReader) {
      readerControlsOpen.value = false;
      await saveCurrentProgress();
      if (!currentRestore(restoreRevision)) return;
      if (readerProgressDirty.value) {
        notify("阅读位置尚未保存，请稍后再试。", "error");
        return;
      }
    }
    closeBookSourceSwitch();
    bookSourceSearchGeneration += 1;
    bookSourceSwitchBookId.value = book.id;
    bookSourceSwitchFromReader.value = fromReader;
    bookSourceChapterTargetId.value = chapterTarget?.id ?? null;
    bookSourceChapterTargetTitle.value = chapterTarget?.title ?? null;
    bookSourceCandidateTaskId.value = null;
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSwitchError.value = "";
    bookSourceChangeError.value = "";
    bookSourceSearchStatus.value = "";
    bookSourceSearchKeyword.value = "";
    bookSourceSearchBusy.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceSwitchOpen.value = true;
    bookSourceSwitchInline.value = inline;
    finishedStreamTaskId = null;
    if (chapterTarget) {
      await startBookSourceCandidateSearch();
    } else {
      const generation = bookSourceSearchGeneration;

      if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration) return;
      bookSourceSearchStatus.value = bookSourceCandidateResults.value.length ? "已缓存的可换来源" : "尚无可换来源，点击刷新搜索。";
    }
  }

  function closeBookSourceSwitch(): void {
    const taskId = bookSourceCandidateTaskId.value;
    const task = taskId ? tasks.value.find((entry) => entry.id === taskId) : undefined;
    const terminalStatuses = ["completed", "failed", "cancelled", "interrupted"];
    const shouldCancel = Boolean(taskId && (bookSourceSearchBusy.value || (task && !terminalStatuses.includes(task.status))));
    bookSourceSearchGeneration += 1;
    bookSourceCandidateTaskId.value = null;
    bookSourceSwitchOpen.value = false;
    bookSourceSwitchInline.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceChapterTargetId.value = null;
    bookSourceChapterTargetTitle.value = null;
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSearchBusy.value = false;
    bookSourceChangeError.value = "";
    if (shouldCancel && taskId) void cancelTask(taskId).catch(() => {});
  }

  async function startBookSourceCandidateSearch(): Promise<void> {
    if (shelfBatchRecoveryRequired.value) return;
    const book = bookSourceSwitchBook.value;
    if (!book || book.canChangeSource !== true || bookSourceSearchBusy.value || bookSourceSwitchBusy.value) return;
    const restoreRevision = getRestoreRevision();
    const generation = ++bookSourceSearchGeneration;
    const sourceIds = bookSourceEligibleSources.value.map((source) => source.id);
    if (!sourceIds.length) {
      bookSourceSwitchError.value = "没有可用于换源的已启用源。";
      return;
    }
    bookSourceSwitchError.value = "";
    chapterSourceError.value = "";
    chapterSourceIdentityConfirmResultId.value = null;
    finishedStreamTaskId = null;
    bookSourceCandidateTaskId.value = null;
    bookSourceSearchStatus.value = "正在搜索其他源…";
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    bookSourceSearchBusy.value = true;
    try {
      const manualKeyword = bookSourceSearchKeyword.value.trim();
      const response = await searchBookSourceCandidates(book.id, sourceIds, manualKeyword || undefined, 1,
        batch => applyCandidateBatch(batch, generation, restoreRevision));
      if (!currentRestore(restoreRevision)) return;
      const taskId = response.taskId ?? response.task?.id;
      if (!taskId) throw new Error("候选搜索没有返回有效任务编号。");
      if (generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) {
        await cancelTask(taskId).catch(() => {});
        return;
      }
      bookSourceCandidateTaskId.value = taskId;
      if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) return;
      await syncBookSourceCandidateTask(response.task);
    } catch (error) {
      if (currentRestore(restoreRevision) && generation === bookSourceSearchGeneration && bookSourceSwitchOpen.value) {
        bookSourceSearchBusy.value = false;
        bookSourceSwitchError.value = errorText(error);
        bookSourceSearchStatus.value = "搜索失败";
      }
    }
  }

  function applyCandidateBatch(batch: BookSourceCandidateBatch, generation: number, restoreRevision: number): void {
    if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) return;
    if (!batch.taskId || (bookSourceCandidateTaskId.value && bookSourceCandidateTaskId.value !== batch.taskId)) return;
    bookSourceCandidateTaskId.value = batch.taskId;
    if (!batch.incremental) {
      searchedCandidates.value = batch.results ?? [];
      bookSourceCandidateErrors.value = batch.errors ?? [];
    } else {
      if (batch.results?.length) searchedCandidates.value = [...searchedCandidates.value, ...batch.results];
      if (batch.errors?.length) bookSourceCandidateErrors.value = [...bookSourceCandidateErrors.value, ...batch.errors];
    }
    bookSourceSearchBusy.value = !batch.complete;
    if (batch.complete) {
      finishedStreamTaskId = batch.taskId;
      bookSourceSearchStatus.value = batch.status === "cancelled" ? "已停止搜索"
        : batch.status === "failed" ? "搜索未能完成" : `找到 ${bookSourceCandidateResults.value.length} 个可用源`;
      if (batch.error) bookSourceSwitchError.value = batch.error;
    } else {
      bookSourceSearchStatus.value = `正在搜索其他源（${batch.completedSources ?? 0} / ${batch.totalSources ?? 0}）`;
    }
  }

  async function stopBookSourceCandidateSearch(): Promise<void> {
    const taskId = bookSourceCandidateTaskId.value;
    if (taskId) {
      try { await cancelTask(taskId); }
      catch (error) { bookSourceSwitchError.value = errorText(error); }
    } else {
      // Admission may still be pending; its eventual response will cancel itself.
      bookSourceSearchGeneration += 1;
      bookSourceSearchBusy.value = false;
      bookSourceSearchStatus.value = "已停止搜索";
    }
  }

  async function syncBookSourceCandidateTask(taskSummary?: Partial<AppTask>): Promise<void> {
    const taskId = bookSourceCandidateTaskId.value;
    if (!taskId || taskId === finishedStreamTaskId) return;
    const task = taskSummary?.id === taskId ? taskSummary : tasks.value.find(entry => entry.id === taskId);
    if (!task || task.kind !== "bookSourceCandidates") return;
    // Results arrive on the channel. Task events only handle pause/failure recovery.
    if (task.status === "paused") {
      bookSourceSearchBusy.value = false;
      bookSourceSearchStatus.value = "候选搜索已暂停";
    } else if (["failed", "interrupted", "recoveryRequired"].includes(task.status ?? "")) {
      bookSourceSearchBusy.value = false;
      bookSourceSearchStatus.value = "候选搜索未能完成";
      bookSourceSwitchError.value = task.error || "候选搜索异常退出，请重新搜索。";
    }
  }

  function selectBookSourceCandidate(candidate: SearchBookResult): void {
    selectedBookSourceCandidate.value = candidate;
    bookSourceIdentityConfirmationNeeded.value = false;
    bookSourceChangeError.value = "";
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
  }

  async function chooseBookSourceCandidate(candidate: SearchBookResult): Promise<void> {
    if (shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value) return;
    selectBookSourceCandidate(candidate);
    if (bookSourceNeedsIdentityConfirmation.value) {
      bookSourceSwitchConfirmOpen.value = true;
      return;
    }
    await confirmBookSourceChange();
    if (bookSourceNeedsIdentityConfirmation.value) bookSourceSwitchConfirmOpen.value = true;
  }

  async function replaceCurrentChapterFromCandidate(candidate: SearchBookResult, confirmMissingAuthor = false): Promise<void> {
    const book = bookSourceSwitchBook.value;
    const chapter = bookSourceCurrentChapter.value;
    const restoreRevision = getRestoreRevision();
    if (!book || shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value) return;
    if (!chapter) {
      const message = bookSourceChapterTargetId.value
        ? `所选章节《${bookSourceChapterTargetTitle.value ?? ""}》已不在当前目录中，请关闭窗口并重新选择章节。`
        : "当前阅读章节已不在目录中，请重新打开书籍详情后再试。";
      chapterSourceError.value = message;
      notify(message, "error");
      return;
    }
    if (candidate.sourceId === book.sourceId) return;
    if (candidate.requiresIdentityConfirmation && !confirmMissingAuthor) {
      chapterSourceIdentityConfirmResultId.value = candidate.resultId;
      chapterSourceError.value = "";
      return;
    }

    const bookId = book.id;
    const chapterIndex = chapter.index;
    const title = book.title;
    const chapterTitle = bookSourceChapterTargetTitle.value ?? chapter.title;
    const readerWasOpen = bookSourceSwitchFromReader.value
      && readerVisible.value
      && readingBook.value?.id === bookId
      && readingChapterIndex.value === chapterIndex;
    chapterSourceBusyResultId.value = candidate.resultId;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSwitchBusy.value = true;
    try {
      const response = await changeChapterSource(bookId, chapter.id, candidate.resultId, confirmMissingAuthor, candidate.alternativeIndex, candidate.sourceId);
      if (!currentRestore(restoreRevision)) return;
      const warning = response.warning?.trim() ?? "";
      const operationError = response.error?.trim() ?? "";

      // 只有明确提交成功才刷新本地书架与阅读器，未知状态统一进入恢复流程。
      if (response.commitState !== "committed") {
        if (response.commitState === "notCommitted"
          && !confirmMissingAuthor
          && operationError.toLocaleLowerCase().includes("author could not be verified")) {
          chapterSourceIdentityConfirmResultId.value = candidate.resultId;
          chapterSourceError.value = "";
          return;
        }
        if (response.commitState === "notCommitted") {
          const message = warning || operationError || "本章正文替换未提交。";
          if (response.recoveryRequired) {
            shelfBatchRecoveryRequired.value = true;
            shelfBatchRecoveryTitle.value = "需要重启应用完成章节恢复";
            shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”未替换，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`;
            chapterSourceError.value = shelfBatchRecoveryMessage.value;
            notify("本章正文替换未提交；恢复待完成，请重启应用。", "error");
          } else {
            chapterSourceError.value = `《${title}》的章节“${chapterTitle}”未替换，可以重试。${message}`;
            notify(chapterSourceError.value, "error");
          }
        } else {
          shelfBatchRecoveryRequired.value = true;
          shelfBatchRecoveryTitle.value = "本章正文替换状态未知";
          shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”替换状态未知。请重启应用检查书籍状态后再继续。${warning || operationError ? ` ${warning || operationError}` : ""}`;
          chapterSourceError.value = shelfBatchRecoveryMessage.value;
          notify("本章正文替换状态未知，已暂停书架修改；请重启应用检查。", "error");
        }
        return;
      }

      if (response.recoveryRequired) {
        shelfBatchRecoveryRequired.value = true;
        shelfBatchRecoveryTitle.value = "需要重启应用完成章节恢复";
        shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”已替换，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`;
        chapterSourceError.value = shelfBatchRecoveryMessage.value;
        notify("本章正文已替换，但恢复待完成；请重启应用。", "error");
      } else {
        chapterSourceError.value = "";
        if (readerWasOpen) closeBookSourceSwitch();
        notify(`《${title}》的章节“${chapterTitle}”正文已替换。${warning ? ` ${warning}` : ""}`);
      }
    } catch (error) {
      if (!currentRestore(restoreRevision)) return;
      const message = errorText(error);
      if (!confirmMissingAuthor && message.toLocaleLowerCase().includes("author could not be verified")) {
        chapterSourceIdentityConfirmResultId.value = candidate.resultId;
        chapterSourceError.value = "";
      } else {
        chapterSourceError.value = `替换《${title}》的章节“${chapterTitle}”正文失败：${message}`;
        notify(chapterSourceError.value, "error");
      }
    } finally {
      if (currentRestore(restoreRevision)) {
        chapterSourceBusyResultId.value = null;
        bookSourceSwitchBusy.value = false;
      }
    }
  }

  async function confirmBookSourceChange(): Promise<void> {
    const restoreRevision = getRestoreRevision();
    if (shelfBatchRecoveryRequired.value) return;
    const book = bookSourceSwitchBook.value;
    const candidate = selectedBookSourceCandidate.value;
    if (!book || book.canChangeSource !== true || !candidate || bookSourceSwitchBusy.value) return;
    const readerWasOpen = bookSourceSwitchFromReader.value && readerVisible.value && readingBook.value?.id === book.id;
    if (readerWasOpen) {
      await saveCurrentProgress();
      if (!currentRestore(restoreRevision)) return;
      if (readerProgressDirty.value) {
        bookSourceSwitchError.value = "阅读位置尚未保存，换源暂未执行。";
        bookSourceSwitchConfirmOpen.value = false;
        return;
      }
    }

    bookSourceSwitchBusy.value = true;
    bookSourceSwitchError.value = "";
    bookSourceChangeError.value = "";
    let response: BookSourceMutationResponse;
    let cachedSourceBookId = "";
    const replaceOnShelf = libraryType(book.id) !== undefined;
    try {
      const cachedCandidate = candidate.alternativeIndex !== undefined
        ? await prepareAlternativeBook(book.id, candidate.alternativeIndex, candidate.sourceId)
        : await prepareSearchResultBook(candidate.resultId);
      if (!currentRestore(restoreRevision)) return;
      const cachedBook = await loadBookPreview(cachedCandidate.book);
      if (!cachedBook) return;
      cachedSourceBookId = cachedBook.id;
      if (!currentRestore(restoreRevision)) return;
      response = await changeBookSource(book.id, cachedBook.id, bookSourceNeedsIdentityConfirmation.value);
      if (!currentRestore(restoreRevision)) return;
    } catch (error) {
      if (!currentRestore(restoreRevision)) return;
      bookSourceSwitchBusy.value = false;
      const message = errorText(error);
      if (message.toLocaleLowerCase().includes("author could not be verified")) {
        bookSourceIdentityConfirmationNeeded.value = true;
        bookSourceChangeError.value = "无法核实作者。请确认书名对应的是同一本书，再继续更换。";
      } else {
        bookSourceChangeError.value = message;
      }
      return;
    }

    // Rust 已提交整书换源；先关闭确认框，避免资源读取暂时失败导致重复提交。
    closeBookSourceSwitch();
    bookSourceIdentityConfirmationNeeded.value = false;
    try {
      if (openedBook.value?.id === book.id) {
        try {
          const current = await switchOpenedBook(response.book);
          if (!current || !currentRestore(restoreRevision)) return;
        } catch (error) {
          if (!currentRestore(restoreRevision)) return;
          if (readerWasOpen) {
            readerVisible.value = false;
            readingContent.value = null;
          }
          clearOpenedBook();
          notify(`已切换到“${candidate.sourceName}”，但新目录暂时无法读取：${errorText(error)}`, "error");
          return;
        }
      }
      if (replaceOnShelf && openedBook.value) replaceLibraryContent(book.id, openedBook.value);
      searchedCandidates.value = [];
      bookSourceCandidateErrors.value = [];
      bookSourceSwitchBookId.value = cachedSourceBookId;
      if (readerWasOpen && readingBook.value?.id === cachedSourceBookId) {
        readingChapterIndex.value = response.progress.chapterIndex;
        readerPageIndex.value = response.progress.offset;
        await loadReaderChapter(response.progress.chapterIndex);
        if (!currentRestore(restoreRevision)) return;
      }
      notify(`已切换到“${candidate.sourceName}”。`);
    } catch (error) {
      if (currentRestore(restoreRevision)) notify(`换源已保存，刷新界面失败：${errorText(error)}`, "error");
    } finally {
      if (currentRestore(restoreRevision)) bookSourceSwitchBusy.value = false;
    }
  }
  function resetForRestore(): void {
    // Reset without calling closeBookSourceSwitch: cancelling by an ID from
    // the old library could cancel an unrelated restored task with that ID.
    bookSourceSearchGeneration += 1;
    finishedStreamTaskId = null;
    bookSourceSwitchOpen.value = false;
    bookSourceSwitchInline.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceSwitchBusy.value = false;
    chapterSourceBusyResultId.value = null;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSearchBusy.value = false;
    candidateRetryBusy.value = false;
    bookSourceSwitchFromReader.value = false;
    bookSourceSwitchBookId.value = null;
    bookSourceChapterTargetId.value = null;
    bookSourceChapterTargetTitle.value = null;
    bookSourceCandidateTaskId.value = null;
    searchedCandidates.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    bookSourceSwitchError.value = "";
    bookSourceChangeError.value = "";
    bookSourceSearchStatus.value = "";
    bookSourceSearchKeyword.value = "";
  }

  return {
    resetForRestore,
    bookSourceSwitchOpen,
    bookSourceSwitchInline,
    bookSourceSwitchBusy,
    chapterSourceBusyResultId,
    chapterSourceIdentityConfirmResultId,
    chapterSourceError,
    bookSourceSearchBusy,
    bookSourceSwitchConfirmOpen,
    bookSourceSwitchBook,
    bookSourceEligibleSources,
    bookSourceChapterTargetId,
    bookSourceChapterTargetTitle,
    bookSourceCandidateTaskId,
    bookSourceCandidateResults,
    bookSourceCandidateErrors: computed(() => bookSourceSwitchBookId.value === openedBook.value?.id
      ? bookSourceCandidateErrors.value : []),
    selectedBookSourceCandidate,
    bookSourceSwitchError,
    bookSourceChangeError,
    bookSourceSearchStatus,
    bookSourceSearchKeyword,
    bookSourceCurrentChapter,
    bookSourceNeedsIdentityConfirmation,
    openBookSourceSwitch,
    openBookSourceSwitchForBook,
    closeBookSourceSwitch,
    startBookSourceCandidateSearch,
    stopBookSourceCandidateSearch,
    syncBookSourceCandidateTask,
    selectBookSourceCandidate,
    chooseBookSourceCandidate,
    replaceCurrentChapterFromCandidate,
    confirmBookSourceChange,
  };
}
