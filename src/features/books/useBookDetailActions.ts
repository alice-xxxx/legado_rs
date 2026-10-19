import type { ComputedRef, Ref } from "vue";
import { moreInformation } from "../../api/moreInformation";
import { resetBookProgress } from "../../api/books";
import { type BookResource } from "../../api/types";
import { openedBook, replaceOpenedBook, refreshOpenedBook, clearBookProgress } from "./bookDetailState";
import { refreshContentLibrary } from "./contentLibraryState";

type PendingProgressReset = { id: string; title: string };

export interface BookDetailActionsOptions {
  readingBook: Ref<BookResource | null>;
  panelBusy: Ref<boolean>;
  infoRefreshBusy: Ref<boolean>;
  canRefreshInfo: ComputedRef<boolean>;
  progressResetBusy: Ref<boolean>;
  recoveryRequired: Ref<boolean>;
  pendingProgressReset: Ref<PendingProgressReset | null>;
  progressResetError: Ref<string>;
  readerProgressDirty: Ref<boolean>;
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
    readingBook,
    panelBusy,
    infoRefreshBusy,
    canRefreshInfo,
    progressResetBusy,
    recoveryRequired,
    pendingProgressReset,
    progressResetError,
    readerProgressDirty,
    saveCurrentProgress,
    finishReadingSession,
    closeReader,
    getRestoreRevision,
    notify,
    errorText,
  } = options;

  async function refreshOpenedBookInfo(): Promise<void> {
    const book = openedBook.value;
    if (!book || !book.sourceId || !book.bookUrl || !canRefreshInfo.value || infoRefreshBusy.value || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    infoRefreshBusy.value = true;
    try {
      await moreInformation(book.sourceId, book.bookUrl);
      if (revision !== getRestoreRevision()) return;
      await refreshOpenedBook(book.id);
      notify(`《${book.title}》的信息已刷新。`);
    } catch (error) {
      if (revision === getRestoreRevision()) notify(`刷新书籍信息失败：${errorText(error)}`, "error");
    } finally {
      infoRefreshBusy.value = false;
    }
  }

  async function resetOpenedBookProgress(): Promise<void> {
    const pending = pendingProgressReset.value;
    if (!pending || progressResetBusy.value || panelBusy.value || infoRefreshBusy.value
      || recoveryRequired.value) return;
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
          replaceOpenedBook({ ...openedBook.value, progress: { ...readerBook.progress } });
        }
        await finishReadingSession();
        if (!isCurrentRestore()) return;
        if (readingBook.value?.id === bookId) await closeReader(true);
        if (!isCurrentRestore()) return;
      }

      await resetBookProgress(bookId);
      if (!isCurrentRestore()) return;
      clearBookProgress(bookId);
      pendingProgressReset.value = null;
      await refreshOpenedBook(bookId);
      await refreshContentLibrary();
      if (isCurrentRestore()) notify(`《${title}》的阅读进度已重置。`);
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
