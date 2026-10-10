import { computed, ref, type ComputedRef, type Ref } from "vue";
import { cancelTask } from "../../api/tasks";
import { changeBookSource, changeChapterSource, searchBookSourceCandidates } from "../../api/search";
import { getBook } from "../../api/books";
import { readResource } from "../../api/resources";
import { type AppTask, type BookmarksResource, type BookResource, type BookSourceMutationResponse, type Bookmark, type ResourceDescriptor, type SearchBookResult, type SearchResource, type ShelfResource, type SourceMetadata } from "../../api/types";
import type { PdfPageResource } from "../reader/displayHtml";

interface BookSourceSwitchDependencies {
  /** 应用共享的任务、书架、阅读器和书签状态。 */
  bookSourceCandidates: ComputedRef<SourceMetadata[]>;
  tasks: Ref<AppTask[]>;
  openedBook: Ref<BookResource | null>;
  readingBook: Ref<BookResource | null>;
  readerVisible: Ref<boolean>;
  readingChapterIndex: Ref<number>;
  readerPageIndex: Ref<number>;
  readingChapterHtml: Ref<string>;
  readingChapterRaw: Ref<string>;
  readingPdfPage: Ref<PdfPageResource | null>;
  readingImagePage: Ref<boolean>;
  readerProgressDirty: Ref<boolean>;
  readerControlsOpen: Ref<boolean>;
  shelf: Ref<ShelfResource>;
  bookmarks: Ref<Bookmark[]>;
  shelfBatchRecoveryRequired: Ref<boolean>;
  shelfBatchRecoveryTitle: Ref<string>;
  shelfBatchRecoveryMessage: Ref<string>;
  saveCurrentProgress: () => Promise<void>;
  refreshTasks: () => Promise<void>;
  refreshShelfFromDescriptor: (descriptor: ResourceDescriptor) => Promise<void>;
  loadReaderChapter: (index: number) => Promise<boolean>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  getRestoreRevision: () => number;
}

/** 管理整书换源、单章正文替换和候选任务同步。 */
export function useBookSourceSwitch({
  bookSourceCandidates,
  tasks,
  openedBook,
  readingBook,
  readerVisible,
  readingChapterIndex,
  readerPageIndex,
  readingChapterHtml,
  readingChapterRaw,
  readingPdfPage,
  readingImagePage,
  readerProgressDirty,
  readerControlsOpen,
  shelf,
  bookmarks,
  shelfBatchRecoveryRequired,
  shelfBatchRecoveryTitle,
  shelfBatchRecoveryMessage,
  saveCurrentProgress,
  refreshTasks,
  refreshShelfFromDescriptor,
  loadReaderChapter,
  notify,
  errorText,
  getRestoreRevision,
}: BookSourceSwitchDependencies) {
  const bookSourceSwitchOpen = ref(false);
  const bookSourceSwitchBusy = ref(false);
  const chapterSourceBusyResultId = ref<string | null>(null);
  const chapterSourceIdentityConfirmResultId = ref<string | null>(null);
  const chapterSourceError = ref("");
  const bookSourceSearchBusy = ref(false);
  const candidateRetryBusy = ref(false);
  const bookSourceSwitchConfirmOpen = ref(false);
  const bookSourceSwitchFromReader = ref(false);
  const bookSourceSwitchBook = ref<BookResource | null>(null);
  const bookSourceChapterTargetId = ref<string | null>(null);
  const bookSourceChapterTargetTitle = ref<string | null>(null);
  const bookSourceCandidateTaskId = ref<string | null>(null);
  const bookSourceCandidateResults = ref<SearchBookResult[]>([]);
  const bookSourceCandidateErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
  const selectedBookSourceCandidate = ref<SearchBookResult | null>(null);
  const bookSourceIdentityConfirmationNeeded = ref(false);
  const bookSourceSwitchError = ref("");
  const bookSourceChangeError = ref("");
  const bookSourceSearchStatus = ref("");
  const bookSourceSearchKeyword = ref("");
  const bookSourceCandidateTaskStates = new Set<string>();
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
  }

  async function openBookSourceSwitchForBook(book: BookResource, chapterTarget?: { id: string; title: string }): Promise<void> {
    await openBookSourceSwitchFor(book, false, chapterTarget, getRestoreRevision());
  }

  async function openBookSourceSwitchFor(
    book: BookResource | null,
    fromReader: boolean,
    chapterTarget: { id: string; title: string } | undefined,
    restoreRevision: number,
  ): Promise<void> {
    if (!book || book.canChangeSource !== true || bookSourceSwitchBusy.value || candidateRetryBusy.value) return;
    if (fromReader) {
      readerControlsOpen.value = false;
      await saveCurrentProgress();
      if (!currentRestore(restoreRevision)) return;
      if (readerProgressDirty.value) {
        notify("阅读位置尚未保存，请稍后再试。", "error");
        return;
      }
    }
    bookSourceSearchGeneration += 1;
    bookSourceSwitchBook.value = book;
    bookSourceSwitchFromReader.value = fromReader;
    bookSourceChapterTargetId.value = chapterTarget?.id ?? null;
    bookSourceChapterTargetTitle.value = chapterTarget?.title ?? null;
    bookSourceCandidateTaskId.value = null;
    bookSourceCandidateResults.value = [];
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
    await startBookSourceCandidateSearch();
  }

  function closeBookSourceSwitch(): void {
    const taskId = bookSourceCandidateTaskId.value;
    const task = taskId ? tasks.value.find((entry) => entry.id === taskId) : undefined;
    const terminalStatuses = ["completed", "failed", "cancelled", "interrupted"];
    const shouldCancel = Boolean(taskId && (bookSourceSearchBusy.value || (task && !terminalStatuses.includes(task.status))));
    bookSourceSearchGeneration += 1;
    bookSourceCandidateTaskId.value = null;
    bookSourceSwitchOpen.value = false;
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
    const sourceIds = bookSourceCandidates.value.map((source) => source.id);
    if (!sourceIds.length) {
      bookSourceSwitchError.value = "没有可用于换源的已启用书源。";
      return;
    }
    bookSourceSwitchError.value = "";
    chapterSourceError.value = "";
    chapterSourceIdentityConfirmResultId.value = null;
    bookSourceSearchStatus.value = "正在搜索其他书源…";
    bookSourceCandidateResults.value = [];
    bookSourceCandidateErrors.value = [];
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    bookSourceSearchBusy.value = true;
    try {
      const manualKeyword = bookSourceSearchKeyword.value.trim();
      const response = await searchBookSourceCandidates(book.id, sourceIds, manualKeyword || undefined, 1);
      if (!currentRestore(restoreRevision)) return;
      const taskId = response.taskId ?? response.task?.id;
      if (!taskId) throw new Error("候选搜索没有返回有效任务编号。");
      if (generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value) {
        await cancelTask(taskId).catch(() => {});
        return;
      }
      bookSourceCandidateTaskId.value = taskId;
      await refreshTasks();
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

  async function syncBookSourceCandidateTask(taskSummary?: Partial<AppTask>): Promise<void> {
    const taskId = bookSourceCandidateTaskId.value;
    if (!taskId) return;
    const generation = bookSourceSearchGeneration;
    const restoreRevision = getRestoreRevision();
    const task = tasks.value.find((entry) => entry.id === taskId)
      ?? (taskSummary?.id === taskId ? taskSummary as AppTask : undefined);
    if (!task || task.id !== taskId || task.kind !== "bookSourceCandidates") return;
    if (["queued", "running", "pausing", "paused", "cancelling"].includes(task.status)) {
      bookSourceSearchBusy.value = task.status !== "paused";
      bookSourceSearchStatus.value = task.status === "paused"
        ? "候选搜索已暂停"
        : `正在搜索其他书源${task.total ? `（${task.completed} / ${task.total}）` : "…"}`;
      return;
    }
    if (!["completed", "failed", "cancelled", "interrupted", "recoveryRequired"].includes(task.status)) return;
    const stateKey = `${task.id}:${task.status}`;
    if (bookSourceCandidateTaskStates.has(stateKey)) return;
    bookSourceCandidateTaskStates.add(stateKey);
    bookSourceSearchBusy.value = false;
    if (task.status !== "completed") {
      bookSourceSearchStatus.value = task.status === "recoveryRequired"
        ? "候选搜索状态待恢复"
        : task.status === "cancelled" ? "候选搜索已取消" : "候选搜索未能完成";
      bookSourceSwitchError.value = task.status === "recoveryRequired"
        ? "候选搜索异常退出，任务状态未能完整保存。请重启应用检查后再重新搜索。"
        : task.error ?? (task.status === "cancelled" ? "搜索已取消。" : "请重新搜索候选书源。");
      return;
    }
    // 任务完成状态与这一不可变结果版本由 Rust 一起提交。
    const descriptor = task.result?.resource;
    if (!descriptor) {
      bookSourceSearchStatus.value = "候选资源不可用";
      bookSourceSwitchError.value = "搜索已完成，但候选资源地址缺失。";
      return;
    }
    try {
      const resource = await readResource<SearchResource>(descriptor);
      if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value || bookSourceCandidateTaskId.value !== taskId) return;
      bookSourceCandidateResults.value = Array.isArray(resource.results) ? resource.results : [];
      bookSourceCandidateErrors.value = Array.isArray(resource.errors) ? resource.errors : [];
      bookSourceSearchStatus.value = resource.complete === false ? "候选结果尚未完整" : `找到 ${bookSourceCandidateResults.value.length} 个可用书源`;
      if (resource.complete === false) bookSourceSwitchError.value = "候选结果尚未准备完整，请重新搜索。";
    } catch (error) {
      if (!currentRestore(restoreRevision) || generation !== bookSourceSearchGeneration || !bookSourceSwitchOpen.value || bookSourceCandidateTaskId.value !== taskId) return;
      bookSourceSearchStatus.value = "候选资源读取失败";
      bookSourceSwitchError.value = errorText(error);
    }
  }

  function selectBookSourceCandidate(candidate: SearchBookResult): void {
    selectedBookSourceCandidate.value = candidate;
    bookSourceIdentityConfirmationNeeded.value = false;
    bookSourceChangeError.value = "";
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
  }

  async function replaceCurrentChapterFromCandidate(candidate: SearchBookResult, confirmMissingAuthor = false): Promise<void> {
    const book = bookSourceSwitchBook.value;
    const chapter = bookSourceCurrentChapter.value;
    const restoreRevision = getRestoreRevision();
    if (!book || shelfBatchRecoveryRequired.value || bookSourceSwitchBusy.value || bookSourceSearchBusy.value) return;
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
    const openedBookAtStart = openedBook.value;
    const readingBookAtStart = readingBook.value;
    chapterSourceBusyResultId.value = candidate.resultId;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSwitchBusy.value = true;
    try {
      const response = await changeChapterSource(bookId, chapter.id, candidate.resultId, confirmMissingAuthor);
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

      let updatedBook: BookResource;
      try {
        if (response.chapterId !== chapter.id) throw new Error("返回的章节与所选章节不匹配。");
        updatedBook = await readResource<BookResource>(response.book);
        if (!currentRestore(restoreRevision)) return;
        if (updatedBook.id !== bookId || !updatedBook.chapters.some((entry) => entry.id === chapter.id && entry.resource)) {
          throw new Error("替换后的书籍目录与当前章节不匹配。");
        }
      } catch (error) {
        if (!currentRestore(restoreRevision)) return;
        const responseReadError = errorText(error);
        try {
          updatedBook = await readResource<BookResource>(await getBook(bookId));
          if (!currentRestore(restoreRevision)) return;
          if (updatedBook.id !== bookId || !updatedBook.chapters.some((entry) => entry.id === chapter.id && entry.resource)) {
            throw new Error("重读后的书籍目录与当前章节不匹配。");
          }
        } catch (reloadError) {
          if (!currentRestore(restoreRevision)) return;
          shelfBatchRecoveryRequired.value = true;
          shelfBatchRecoveryTitle.value = "本章正文已提交，显示尚未更新";
          shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”已提交替换，但新目录未能读取。请重启应用后再继续。${responseReadError}；重读失败：${errorText(reloadError)}`;
          chapterSourceError.value = shelfBatchRecoveryMessage.value;
          notify("本章正文已提交，但目录读取失败；请重启应用。", "error");
          return;
        }
      }

      const currentOpenedBook = openedBook.value;
      const currentReadingBook = readingBook.value;
      if ((currentOpenedBook?.id === bookId && currentOpenedBook !== openedBookAtStart)
        || (currentReadingBook?.id === bookId && currentReadingBook !== readingBookAtStart)) {
        try {
          updatedBook = await readResource<BookResource>(await getBook(bookId));
          if (!currentRestore(restoreRevision)) return;
        } catch (error) {
          if (!currentRestore(restoreRevision)) return;
          shelfBatchRecoveryRequired.value = true;
          shelfBatchRecoveryTitle.value = "本章正文已提交，显示尚未更新";
          shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”已提交替换，但并发更新后重读失败。请重启应用后再继续。${errorText(error)}`;
          chapterSourceError.value = shelfBatchRecoveryMessage.value;
          notify("本章正文已提交，但并发更新后重读失败；请重启应用。", "error");
          return;
        }
        if (updatedBook.id !== bookId) {
          shelfBatchRecoveryRequired.value = true;
          shelfBatchRecoveryTitle.value = "本章正文已提交，显示尚未更新";
          shelfBatchRecoveryMessage.value = `《${title}》的章节“${chapterTitle}”已提交替换，但重读到的书籍与当前书籍不匹配。请重启应用后再继续。`;
          chapterSourceError.value = shelfBatchRecoveryMessage.value;
          notify("本章正文已提交，但书籍状态需要重启检查。", "error");
          return;
        }
      }

      if (bookSourceSwitchBook.value?.id === bookId) bookSourceSwitchBook.value = updatedBook;
      if (openedBook.value?.id === bookId) openedBook.value = updatedBook;
      if (readingBook.value?.id === bookId) readingBook.value = updatedBook;

      let readerReadError = "";
      if (readerWasOpen && readerVisible.value && readingChapterIndex.value === chapterIndex) {
        readingChapterRaw.value = "";
        readingChapterHtml.value = "";
        readingPdfPage.value = null;
        readingImagePage.value = false;
        await loadReaderChapter(chapterIndex);
        if (!currentRestore(restoreRevision)) return;
        if (!readingChapterRaw.value) readerReadError = "替换后的章节内容未能重新载入。";
      }

      if (response.recoveryRequired || readerReadError) {
        shelfBatchRecoveryRequired.value = true;
        shelfBatchRecoveryTitle.value = "需要重启应用完成章节恢复";
        shelfBatchRecoveryMessage.value = readerReadError
          ? `《${title}》的章节“${chapterTitle}”已替换，但阅读内容未能重新载入。请重启应用后再继续。${warning ? ` ${warning}` : ""}`
          : `《${title}》的章节“${chapterTitle}”已替换，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`;
        chapterSourceError.value = shelfBatchRecoveryMessage.value;
        notify(readerReadError ? "本章正文已替换，但阅读内容未能重新载入；请重启应用。" : "本章正文已替换，但恢复待完成；请重启应用。", "error");
      } else {
        chapterSourceError.value = "";
        if (readerWasOpen) closeBookSourceSwitch();
        notify(`《${title}》的章节“${chapterTitle}”正文已替换并重新载入。${warning ? ` ${warning}` : ""}`);
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
    try {
      response = await changeBookSource(book.id, candidate.resultId, bookSourceNeedsIdentityConfirmation.value);
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
    bookSourceSwitchOpen.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceCandidateTaskId.value = null;
    selectedBookSourceCandidate.value = null;
    bookSourceIdentityConfirmationNeeded.value = false;
    let updatedBook: BookResource;
    try {
      try {
        updatedBook = await readResource<BookResource>(response.book);
        if (!currentRestore(restoreRevision)) return;
      } catch {
        if (!currentRestore(restoreRevision)) return;
        updatedBook = await readResource<BookResource>(await getBook(book.id));
        if (!currentRestore(restoreRevision)) return;
      }
    } catch (error) {
      if (!currentRestore(restoreRevision)) return;
      bookSourceSwitchBusy.value = false;
      if (readerWasOpen) {
        readingBook.value = null;
        readerVisible.value = false;
        readingChapterHtml.value = "";
        readingChapterRaw.value = "";
        readingPdfPage.value = null;
      }
      openedBook.value = null;
      void refreshShelfFromDescriptor(response.shelf).catch(() => {});
      notify(`已切换到“${candidate.sourceName}”，但新目录暂时无法读取：${errorText(error)}`, "error");
      return;
    }

    updatedBook.progress = { ...response.progress };
    try {
      const nextShelf = await readResource<ShelfResource>(response.shelf);
      if (!currentRestore(restoreRevision)) return;
      shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
    } catch (error) {
      if (!currentRestore(restoreRevision)) return;
      notify(`书源已切换，书架更新稍后重试：${errorText(error)}`, "error");
    }
    try {
      const bookmarkResource = await readResource<BookmarksResource>(response.bookmarks.resource);
      if (!currentRestore(restoreRevision)) return;
      bookmarks.value = bookmarkResource.bookmarks ?? [];
    } catch (error) {
      if (!currentRestore(restoreRevision)) return;
      notify(`书源已切换，书签列表读取失败：${errorText(error)}`, "error");
    }

    openedBook.value = openedBook.value?.id === book.id ? updatedBook : openedBook.value;
    // Candidate matches were computed for the previous source and directory.
    // Once the Rust source mutation commits, do not present them as current matches.
    bookSourceCandidateResults.value = [];
    bookSourceCandidateErrors.value = [];
    bookSourceSwitchBook.value = updatedBook;
    if (readerWasOpen) {
      readingBook.value = updatedBook;
      readingPdfPage.value = null;
      readingChapterHtml.value = "";
      readingChapterRaw.value = "";
      readingChapterIndex.value = response.progress.chapterIndex;
      readerPageIndex.value = response.progress.offset;
      await loadReaderChapter(response.progress.chapterIndex);
      if (!currentRestore(restoreRevision)) return;
    }
    const progressMessage = "阅读位置已按新目录更新";
    const bookmarkMessage = response.bookmarks.orphanedCount
      ? `，${response.bookmarks.orphanedCount} 个未匹配书签仍保留在列表中`
      : "";
    bookSourceSwitchBusy.value = false;
    notify(`已切换到“${candidate.sourceName}”；${progressMessage}${bookmarkMessage}。`);
  }
  function resetForRestore(): void {
    // Reset without calling closeBookSourceSwitch: cancelling by an ID from
    // the old library could cancel an unrelated restored task with that ID.
    bookSourceSearchGeneration += 1;
    bookSourceCandidateTaskStates.clear();
    bookSourceSwitchOpen.value = false;
    bookSourceSwitchConfirmOpen.value = false;
    bookSourceSwitchBusy.value = false;
    chapterSourceBusyResultId.value = null;
    chapterSourceIdentityConfirmResultId.value = null;
    chapterSourceError.value = "";
    bookSourceSearchBusy.value = false;
    candidateRetryBusy.value = false;
    bookSourceSwitchFromReader.value = false;
    bookSourceSwitchBook.value = null;
    bookSourceChapterTargetId.value = null;
    bookSourceChapterTargetTitle.value = null;
    bookSourceCandidateTaskId.value = null;
    bookSourceCandidateResults.value = [];
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
    bookSourceSwitchBusy,
    chapterSourceBusyResultId,
    chapterSourceIdentityConfirmResultId,
    chapterSourceError,
    bookSourceSearchBusy,
    bookSourceSwitchConfirmOpen,
    bookSourceSwitchBook,
    bookSourceChapterTargetId,
    bookSourceChapterTargetTitle,
    bookSourceCandidateTaskId,
    bookSourceCandidateResults,
    bookSourceCandidateErrors,
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
    syncBookSourceCandidateTask,
    selectBookSourceCandidate,
    replaceCurrentChapterFromCandidate,
    confirmBookSourceChange,
  };
}
