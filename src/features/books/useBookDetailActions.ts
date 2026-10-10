import type { ComputedRef, Ref } from "vue";
import { getBook, refreshBookInfo, resetBookProgress } from "../../api/books";
import { readResource } from "../../api/resources";
import { type BookResource, type ReadingProgress, type ResourceDescriptor, type ResetBookProgressResponse, type ShelfResource } from "../../api/types";

type PendingProgressReset = { id: string; title: string };

export interface BookDetailActionsOptions {
  openedBook: Ref<BookResource | null>;
  readingBook: Ref<BookResource | null>;
  shelf: Ref<ShelfResource>;
  panelBusy: Ref<boolean>;
  infoRefreshBusy: Ref<boolean>;
  canRefreshInfo: ComputedRef<boolean>;
  progressResetBusy: Ref<boolean>;
  displaySaveBusy: Ref<boolean>;
  singleRemovalBusy: Ref<boolean>;
  recoveryRequired: Ref<boolean>;
  recoveryTitle: Ref<string>;
  recoveryMessage: Ref<string>;
  pendingProgressReset: Ref<PendingProgressReset | null>;
  progressResetError: Ref<string>;
  readerProgressDirty: Ref<boolean>;
  refreshShelfFromDescriptor: (descriptor: ResourceDescriptor) => Promise<void>;
  saveCurrentProgress: () => Promise<void>;
  finishReadingSession: () => Promise<void>;
  closeReader: (discardProgress?: boolean) => Promise<void>;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理书籍详情载入、信息刷新和阅读进度重置。 */
export function useBookDetailActions(options: BookDetailActionsOptions) {
  const {
    openedBook,
    readingBook,
    shelf,
    panelBusy,
    infoRefreshBusy,
    canRefreshInfo,
    progressResetBusy,
    displaySaveBusy,
    singleRemovalBusy,
    recoveryRequired,
    recoveryTitle,
    recoveryMessage,
    pendingProgressReset,
    progressResetError,
    readerProgressDirty,
    refreshShelfFromDescriptor,
    saveCurrentProgress,
    finishReadingSession,
    closeReader,
    getRestoreRevision,
    notify,
    errorText,
  } = options;

  async function refreshOpenedBookInfo(): Promise<void> {
    const book = openedBook.value;
    if (!book || !canRefreshInfo.value || infoRefreshBusy.value || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    const bookAtStart = book;
    const bookId = book.id;
    infoRefreshBusy.value = true;
    try {
      const response = await refreshBookInfo(bookId);
      if (revision !== getRestoreRevision()) return;
      const warning = response.warning?.trim() ?? "";
      if (response.commitState !== "committed") {
        if (response.commitState === "notCommitted") {
          if (response.recoveryRequired) {
            recoveryRequired.value = true;
            recoveryTitle.value = "需要重启应用完成恢复";
            recoveryMessage.value = `《${book.title}》的信息刷新未提交，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`;
            notify("信息刷新未提交；恢复待完成，请重启应用。", "error");
          } else {
            notify(`《${book.title}》的信息刷新未提交，可以重试。${warning ? ` ${warning}` : ""}`, "error");
          }
        } else {
          recoveryRequired.value = true;
          recoveryTitle.value = "书籍信息刷新状态未知";
          recoveryMessage.value = `《${book.title}》的信息刷新提交状态未知。请重启应用检查书籍状态后再继续。${warning ? ` ${warning}` : ""}`;
          notify("信息刷新状态未知，已暂停书架修改；请重启应用检查。", "error");
        }
        return;
      }

      let refreshedBook: BookResource | null = null;
      let bookReadError = "";
      try {
        refreshedBook = await readResource<BookResource>(response.book);
        if (refreshedBook.id !== bookId) throw new Error("刷新的书籍与当前详情不匹配。");
      } catch (error) {
        const responseReadError = errorText(error);
        try {
          refreshedBook = await readResource<BookResource>(await getBook(bookId));
          if (refreshedBook.id !== bookId) throw new Error("重读后返回的书籍与当前详情不匹配。");
        } catch (reloadError) {
          bookReadError = `${responseReadError}；重读失败：${errorText(reloadError)}`;
        }
      }

      let shelfReadError = "";
      if (revision === getRestoreRevision()) {
        try {
          await refreshShelfFromDescriptor(response.shelf);
        } catch (error) {
          shelfReadError = errorText(error);
        }
      }

      const current = openedBook.value;
      if (revision === getRestoreRevision() && refreshedBook && current?.id === bookId) {
        if (current !== bookAtStart) {
          try {
            refreshedBook = await readResource<BookResource>(await getBook(bookId));
            if (refreshedBook.id !== bookId) throw new Error("重读后返回的书籍与当前详情不匹配。");
          } catch (error) {
            bookReadError = `并发更新后重读失败：${errorText(error)}`;
            refreshedBook = null;
          }
        }
        if (revision === getRestoreRevision() && refreshedBook && openedBook.value === current) openedBook.value = refreshedBook;
      }

      if (response.recoveryRequired && revision === getRestoreRevision()) {
        recoveryRequired.value = true;
        recoveryTitle.value = "需要重启应用完成恢复";
        recoveryMessage.value = `《${book.title}》的信息已刷新，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`;
      }

      if (revision !== getRestoreRevision()) return;
      if (bookReadError || shelfReadError) {
        const details = [bookReadError && `详情未能重新载入：${bookReadError}`, shelfReadError && `书架未能更新：${shelfReadError}`].filter(Boolean).join("；");
        notify(`书籍信息已刷新，但${details}。`, "error");
      } else if (response.recoveryRequired) {
        notify("书籍信息已刷新，但恢复待完成；请重启应用。", "error");
      } else {
        notify(`书籍信息已刷新。${warning ? ` ${warning}` : ""}`);
      }
    } catch (error) {
      if (revision === getRestoreRevision()) notify(`刷新书籍信息失败：${errorText(error)}`, "error");
    } finally {
      infoRefreshBusy.value = false;
    }
  }

  async function resetOpenedBookProgress(): Promise<void> {
    const pending = pendingProgressReset.value;
    if (!pending || progressResetBusy.value || panelBusy.value || displaySaveBusy.value || infoRefreshBusy.value
      || singleRemovalBusy.value || recoveryRequired.value) return;
    const { id: bookId, title } = pending;
    const restoreRevision = getRestoreRevision();
    const isCurrentRestore = () => restoreRevision === getRestoreRevision();
    progressResetBusy.value = true;
    progressResetError.value = "";
    try {
      const readerBook = readingBook.value?.id === bookId ? readingBook.value : null;
      if (readerBook) {
        await saveCurrentProgress();
        if (!isCurrentRestore()) return;
        if (readingBook.value?.id !== bookId) {
          progressResetError.value = `阅读器中的书籍已改变，未重置《${title}》的阅读进度。请重新确认后再试。`;
          notify(progressResetError.value, "error");
          return;
        }
        if (readerProgressDirty.value) {
          progressResetError.value = `《${title}》的阅读位置尚未保存，重置已取消。请保留当前状态后重试。`;
          notify(progressResetError.value, "error");
          return;
        }
        if (openedBook.value?.id === bookId && readerBook.progress) {
          openedBook.value = { ...openedBook.value, progress: { ...readerBook.progress } };
        }
        if (readerBook.progress) {
          shelf.value = {
            ...shelf.value,
            books: shelf.value.books.map((book) => book.id === bookId ? { ...book, progress: { ...readerBook.progress! } } : book),
          };
        }
        await finishReadingSession();
        if (!isCurrentRestore()) return;
        if (readingBook.value?.id === bookId) await closeReader(true);
        if (!isCurrentRestore()) return;
      }

      let response: ResetBookProgressResponse;
      try {
        response = await resetBookProgress(bookId);
      } catch (error) {
        if (!isCurrentRestore()) return;
        const message = errorText(error);
        pendingProgressReset.value = null;
        recoveryRequired.value = true;
        recoveryTitle.value = "阅读进度重置状态未确认";
        recoveryMessage.value = `没有收到《${title}》的结构化重置结果，无法确认阅读进度是否已清除。请重启应用检查书籍状态后再继续。应用返回信息：${message}`;
        notify("阅读进度重置状态未知，已暂停书架修改；请重启应用检查。", "error");
        return;
      }

      if (!isCurrentRestore()) return;
      const warning = response.warning?.trim() ?? "";
      if (response.commitState !== "committed") {
        if (response.commitState === "notCommitted") {
          if (response.recoveryRequired) {
            pendingProgressReset.value = null;
            recoveryRequired.value = true;
            recoveryTitle.value = "需要重启应用完成进度恢复";
            recoveryMessage.value = `《${title}》的阅读进度未重置，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`;
            notify("阅读进度未重置；恢复待完成，请重启应用。", "error");
          } else {
            progressResetError.value = `《${title}》的阅读进度未重置，可以重试。${warning || "操作未提交，原进度仍保留。"}`;
            notify(progressResetError.value, "error");
          }
        } else {
          pendingProgressReset.value = null;
          recoveryRequired.value = true;
          recoveryTitle.value = "阅读进度重置状态未知";
          recoveryMessage.value = `《${title}》的阅读进度重置提交状态未知。请重启应用检查书籍状态后再继续。${warning ? ` ${warning}` : ""}`;
          notify("阅读进度重置状态未知，已暂停书架修改；请重启应用检查。", "error");
        }
        return;
      }

      pendingProgressReset.value = null;
      const resetProgress: ReadingProgress = { chapterId: null, chapterIndex: 0, offset: 0, updatedAtMs: 0 };
      if (openedBook.value?.id === bookId) openedBook.value = { ...openedBook.value, progress: resetProgress };
      shelf.value = {
        ...shelf.value,
        books: shelf.value.books.map((book) => book.id === bookId ? { ...book, progress: resetProgress } : book),
      };

      let bookReadError = "";
      try {
        const updatedBook = await readResource<BookResource>(response.book);
        if (!isCurrentRestore()) return;
        if (updatedBook.id !== bookId) throw new Error("重置后返回的书籍与当前操作不匹配。");
        if (openedBook.value?.id === bookId) openedBook.value = updatedBook;
      } catch (error) {
        const responseReadError = errorText(error);
        try {
          const updatedBook = await readResource<BookResource>(await getBook(bookId));
          if (!isCurrentRestore()) return;
          if (updatedBook.id !== bookId) throw new Error("重读后返回的书籍与当前操作不匹配。");
          if (openedBook.value?.id === bookId) openedBook.value = updatedBook;
        } catch (reloadError) {
          if (!isCurrentRestore()) return;
          bookReadError = `${responseReadError}；重读失败：${errorText(reloadError)}`;
        }
      }

      if (!isCurrentRestore()) return;
      let shelfReadError = "";
      try {
        await refreshShelfFromDescriptor(response.shelf);
      } catch (error) {
        if (!isCurrentRestore()) return;
        shelfReadError = errorText(error);
      }

      if (!isCurrentRestore()) return;
      if (response.recoveryRequired) {
        recoveryRequired.value = true;
        recoveryTitle.value = "需要重启应用完成进度恢复";
        recoveryMessage.value = `《${title}》的阅读进度已重置，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`;
      }

      if (bookReadError || shelfReadError) {
        const details = [bookReadError && `详情未能重新载入：${bookReadError}`, shelfReadError && `书架未能更新：${shelfReadError}`].filter(Boolean).join("；");
        notify(`《${title}》的阅读进度已重置；${details}。`, "error");
      } else if (response.recoveryRequired) {
        notify("阅读进度已重置，但恢复待完成；请重启应用。", "error");
      } else {
        notify(`《${title}》的阅读进度已重置。目录、缓存、书源、书签和阅读历史已保留。${warning ? ` ${warning}` : ""}`);
      }
    } catch (error) {
      if (isCurrentRestore()) {
        progressResetError.value = errorText(error);
        notify(`重置《${title}》的阅读进度失败：${errorText(error)}`, "error");
      }
    } finally {
      progressResetBusy.value = false;
    }
  }

  return {
    refreshOpenedBookInfo,
    resetOpenedBookProgress,
  };
}
