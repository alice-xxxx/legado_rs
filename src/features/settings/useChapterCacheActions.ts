import { watch, type Ref } from "vue";
import { clearBookChapterCache, getBook, getChapterCacheUsage } from "../../api/books";
import { readResource } from "../../api/resources";
import { type BookResource, type ChapterCacheUsage, type ClearBookChapterCacheResponse, type ShelfResource } from "../../api/types";

type ClearResult = {
  clearedBooks: number;
  failedBooks: number;
  notStartedBooks: number;
  clearedChapters: number;
  errors: Array<{ bookId: string; title: string; message: string }>;
};

interface ChapterCacheActionOptions {
  shelf: Ref<ShelfResource>;
  openedBook: Ref<BookResource | null>;
  readingBook: Ref<BookResource | null>;
  readerVisible: Ref<boolean>;
  readerProgressDirty: Ref<boolean>;
  chapterCacheUsage: Ref<ChapterCacheUsage | null>;
  chapterCacheUsageBusy: Ref<boolean>;
  chapterCacheUsageError: Ref<string>;
  pendingShelfClear: Ref<boolean>;
  shelfClearBusy: Ref<boolean>;
  shelfClearResult: Ref<ClearResult | null>;
  pendingBookClear: Ref<{ id: string; title: string } | null>;
  bookClearBusy: Ref<boolean>;
  bookClearError: Ref<string>;
  recoveryRequired: Ref<boolean>;
  recoveryTitle: Ref<string>;
  recoveryMessage: Ref<string>;
  screen: { readonly value: string };
  bootstrapped: Ref<boolean>;
  hasConflictingOperation: () => boolean;
  getRestoreRevision: () => number;
  saveCurrentProgress: () => Promise<void>;
  finishReadingSession: () => Promise<void>;
  closeReader: (force: boolean) => Promise<void>;
  errorText: (error: unknown) => string;
  notify: (message: string, kind?: "success" | "error") => void;
}

/** 集中处理章节缓存统计、书架批量清理和详情页缓存清理。 */
export function useChapterCacheActions(options: ChapterCacheActionOptions) {
  const {
    shelf, openedBook, readingBook, readerVisible, readerProgressDirty,
    chapterCacheUsage, chapterCacheUsageBusy, chapterCacheUsageError,
    pendingShelfClear, shelfClearBusy, shelfClearResult,
    pendingBookClear, bookClearBusy, bookClearError,
    recoveryRequired, recoveryTitle, recoveryMessage,
    screen, bootstrapped, hasConflictingOperation, getRestoreRevision,
    saveCurrentProgress, finishReadingSession, closeReader, errorText, notify,
  } = options;

  // Only a successfully reloaded Rust book snapshot may restore published
  // chapter resources after cache clearing. The fallback never claims that
  // an obsolete descriptor is still available.
  function withoutChapterResources(book: BookResource): BookResource {
    return {
      ...book,
      chapters: book.chapters.map((chapter) => ({
        ...chapter,
        resource: undefined,
      })),
    };
  }

  function lockChapterCacheRecovery(title: string, message: string): void {
    recoveryRequired.value = true;
    recoveryTitle.value = title;
    recoveryMessage.value = message;
  }

  async function prepareReaderForChapterCacheClear(bookId: string, title: string): Promise<string | null> {
    const revision = getRestoreRevision();
    const isCurrent = () => revision === getRestoreRevision();
    const readerBook = readingBook.value?.id === bookId ? readingBook.value : null;
    if (!readerBook) return null;
    try {
      await saveCurrentProgress();
      if (!isCurrent()) return "备份恢复期间书架已变化，旧缓存清理已停止。";
      if (readingBook.value?.id !== bookId) return `阅读器中的书籍已改变，未清除《${title}》的章节缓存。请重新确认后再试。`;
      if (readerProgressDirty.value) return `《${title}》的阅读位置尚未保存，缓存未清除。请保留当前状态后重试。`;
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
      if (!isCurrent()) return "备份恢复期间书架已变化，旧缓存清理已停止。";
      if (readingBook.value?.id === bookId) await closeReader(true);
      if (!isCurrent()) return "备份恢复期间书架已变化，旧缓存清理已停止。";
      if (readingBook.value?.id === bookId || readerVisible.value) return `阅读器尚未关闭，未清除《${title}》的章节缓存。请关闭阅读器后重试。`;
      return null;
    } catch (error) {
      return `保存阅读位置或关闭阅读器失败，未清除《${title}》的章节缓存：${errorText(error)}`;
    }
  }

  async function refreshChapterCacheUsage(): Promise<void> {
    if (chapterCacheUsageBusy.value || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    const isCurrent = () => revision === getRestoreRevision();
    chapterCacheUsageBusy.value = true;
    chapterCacheUsageError.value = "";
    try {
      const usage = await getChapterCacheUsage();
      if (isCurrent()) chapterCacheUsage.value = usage;
    } catch (error) {
      if (isCurrent()) chapterCacheUsageError.value = errorText(error);
    } finally {
      if (isCurrent()) chapterCacheUsageBusy.value = false;
    }
  }

  watch(() => [screen.value, bootstrapped.value, recoveryRequired.value] as const, ([currentScreen, ready, locked]) => {
    if (ready && currentScreen === "settings" && !locked && !chapterCacheUsage.value && !chapterCacheUsageBusy.value) {
      void refreshChapterCacheUsage();
    }
  }, { immediate: true });

  function formatChapterCacheBytes(bytes: number): string {
    const safeBytes = Number.isFinite(bytes) ? Math.max(0, bytes) : 0;
    if (safeBytes < 1024) return `${Math.round(safeBytes)} B`;
    const units = ["KiB", "MiB", "GiB", "TiB"];
    let value = safeBytes;
    let unit = "B";
    for (const nextUnit of units) {
      value /= 1024;
      unit = nextUnit;
      if (value < 1024 || nextUnit === "TiB") break;
    }
    return `${value.toFixed(value >= 100 ? 0 : value >= 10 ? 1 : 2)} ${unit}`;
  }

  async function clearShelfOnlineChapterCaches(): Promise<void> {
    if (!pendingShelfClear.value || chapterCacheUsageBusy.value || shelfClearBusy.value || bookClearBusy.value
      || hasConflictingOperation() || recoveryRequired.value) return;

    const revision = getRestoreRevision();
    const isCurrent = () => revision === getRestoreRevision();
    pendingShelfClear.value = false;
    shelfClearBusy.value = true;
    shelfClearResult.value = null;
    let clearedBooks = 0;
    let failedBooks = 0;
    let notStartedBooks = 0;
    let clearedChapters = 0;
    let stoppedForRecovery = false;
    let readerPreparationFailed = false;
    const errors: ClearResult["errors"] = [];
    const shelfBooksAtStart = [...(shelf.value.books ?? [])];
    const onlineBooks: BookResource[] = [];

    try {
      for (let index = 0; index < shelfBooksAtStart.length; index += 1) {
        if (!isCurrent()) return;
        const shelfBook = shelfBooksAtStart[index];
        if (recoveryRequired.value) {
          stoppedForRecovery = true;
          notStartedBooks += onlineBooks.length + shelfBooksAtStart.length - index;
          errors.push({ bookId: shelfBook.id, title: shelfBook.title, message: "恢复状态已锁定，无法继续检查书籍或清理缓存。" });
          break;
        }
        try {
          const book = await readResource<BookResource>(await getBook(shelfBook.id));
          if (!isCurrent()) return;
          if (book.id !== shelfBook.id) throw new Error("返回的书籍与书架条目不匹配。");
          if (book.canChangeSource === true && Boolean(book.sourceId?.trim())) onlineBooks.push(book);
        } catch (error) {
          if (!isCurrent()) return;
          failedBooks += 1;
          errors.push({ bookId: shelfBook.id, title: shelfBook.title, message: `无法确认书籍类型，未清除缓存：${errorText(error)}` });
        }
      }

      if (!stoppedForRecovery) {
        const currentReaderTarget = onlineBooks.find((book) => book.id === readingBook.value?.id);
        if (currentReaderTarget) {
          const readerError = await prepareReaderForChapterCacheClear(currentReaderTarget.id, currentReaderTarget.title);
          if (!isCurrent()) return;
          if (readerError) {
            failedBooks += 1;
            errors.push({ bookId: currentReaderTarget.id, title: currentReaderTarget.title, message: readerError });
            notStartedBooks += Math.max(0, onlineBooks.length - 1);
            readerPreparationFailed = true;
            stoppedForRecovery = recoveryRequired.value;
          }
        }
      }

      if (!stoppedForRecovery && !readerPreparationFailed) {
        for (let index = 0; index < onlineBooks.length; index += 1) {
          if (!isCurrent()) return;
          const book = onlineBooks[index];
          if (recoveryRequired.value) {
            stoppedForRecovery = true;
            notStartedBooks += onlineBooks.length - index;
            break;
          }
          let response: ClearBookChapterCacheResponse;
          try {
            response = await clearBookChapterCache(book.id);
            if (!isCurrent()) return;
          } catch (error) {
            if (!isCurrent()) return;
            failedBooks += 1;
            errors.push({ bookId: book.id, title: book.title, message: `清理状态未知：${errorText(error)}` });
            lockChapterCacheRecovery("章节缓存清除状态未知", `没有收到《${book.title}》的结构化清除结果，无法确认章节缓存是否已清除。请重启应用检查书籍状态后再继续。应用返回信息：${errorText(error)}`);
            stoppedForRecovery = true;
            notStartedBooks += onlineBooks.length - index - 1;
            break;
          }
          const warning = response.warning?.trim() ?? "";
          if (response.commitState === "committed") {
            clearedBooks += 1;
            clearedChapters += response.clearedCount;
          } else {
            failedBooks += 1;
            const message = response.commitState === "notCommitted" ? response.error?.trim() : "";
            errors.push({ bookId: book.id, title: book.title, message: message || warning || "章节正文缓存未清除。" });
          }
          if (response.commitState === "indeterminate" || response.recoveryRequired) {
            const title = response.commitState === "indeterminate" ? "章节缓存清除状态未知" : "需要重启应用完成缓存恢复";
            const detail = response.commitState === "indeterminate"
              ? `《${book.title}》的章节缓存清除提交状态未知。请重启应用检查书籍状态后再继续。`
              : `《${book.title}》的章节缓存${response.commitState === "committed" ? "已清除" : "未清除"}，但回滚、清理或恢复尚未完成。请重启应用后再继续。`;
            lockChapterCacheRecovery(title, `${detail}${warning ? ` ${warning}` : ""}`);
            stoppedForRecovery = true;
            notStartedBooks += onlineBooks.length - index - 1;
            break;
          }
        }
      }

      if (!isCurrent()) return;
      if (recoveryRequired.value) stoppedForRecovery = true;
      shelfClearResult.value = { clearedBooks, failedBooks, notStartedBooks, clearedChapters, errors };
      if (!recoveryRequired.value) await refreshChapterCacheUsage();
      if (!isCurrent()) return;
      const summary = `正文缓存清理结果：成功 ${clearedBooks} 本（${clearedChapters} 章），失败 ${failedBooks} 本，未开始 ${notStartedBooks} 本。`;
      notify(stoppedForRecovery ? `${summary}已暂停后续清理，请重启应用恢复。` : summary, failedBooks || notStartedBooks || stoppedForRecovery ? "error" : "success");
    } finally {
      if (isCurrent()) shelfClearBusy.value = false;
    }
  }

  async function clearOpenedBookChapterCache(): Promise<void> {
    const pending = pendingBookClear.value;
    if (!pending || bookClearBusy.value || hasConflictingOperation() || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    const isCurrent = () => revision === getRestoreRevision();
    const { id: bookId, title } = pending;
    const bookAtStart = openedBook.value;
    if (!bookAtStart || bookAtStart.id !== bookId || bookAtStart.canChangeSource !== true || !bookAtStart.sourceId) {
      bookClearError.value = `《${title}》不是可在线获取的书籍，不能清除章节缓存。`;
      notify(bookClearError.value, "error");
      return;
    }

    bookClearBusy.value = true;
    bookClearError.value = "";
    try {
      const readerPreparationError = await prepareReaderForChapterCacheClear(bookId, title);
      if (!isCurrent()) return;
      if (readerPreparationError) {
        bookClearError.value = readerPreparationError;
        notify(bookClearError.value, "error");
        return;
      }
      let response: ClearBookChapterCacheResponse;
      try {
        response = await clearBookChapterCache(bookId);
        if (!isCurrent()) return;
      } catch (error) {
        if (!isCurrent()) return;
        const message = errorText(error);
        pendingBookClear.value = null;
        lockChapterCacheRecovery("章节缓存清除状态未确认", `没有收到《${title}》的结构化清除结果，无法确认章节缓存是否已清除。请重启应用检查书籍状态后再继续。应用返回信息：${message}`);
        notify("章节缓存清除状态未知，已暂停书架修改；请重启应用检查。", "error");
        return;
      }

      const warning = response.warning?.trim() ?? "";
      if (response.commitState === "notCommitted") {
        if (response.recoveryRequired) {
          pendingBookClear.value = null;
          lockChapterCacheRecovery("需要重启应用完成缓存恢复", `《${title}》的章节缓存未清除，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`);
          notify("章节缓存未清除；恢复待完成，请重启应用。", "error");
        } else {
          bookClearError.value = `《${title}》的章节缓存未清除，可以重试。${response.error || warning || "操作未提交，原缓存仍保留。"}`;
          notify(bookClearError.value, "error");
        }
        return;
      }
      if (response.commitState === "indeterminate") {
        pendingBookClear.value = null;
        lockChapterCacheRecovery("章节缓存清除状态未知", `《${title}》的章节缓存清除提交状态未知。请重启应用检查书籍状态后再继续。${warning ? ` ${warning}` : ""}`);
        notify("章节缓存清除状态未知，已暂停书架修改；请重启应用检查。", "error");
        return;
      }
      if (response.recoveryRequired || recoveryRequired.value) {
        pendingBookClear.value = null;
        if (openedBook.value?.id === bookId) {
          openedBook.value = withoutChapterResources(openedBook.value);
        }
        if (response.recoveryRequired) lockChapterCacheRecovery("需要重启应用完成缓存恢复", `《${title}》的章节缓存已清除，但清理或恢复尚未完成。请重启应用。${warning ? ` ${warning}` : ""}`);
        notify(`《${title}》的缓存清理已提交，但当前需要完成恢复；未刷新书籍资源，请重启应用。${warning ? ` ${warning}` : ""}`, "error");
        return;
      }

      pendingBookClear.value = null;
      let updatedBook: BookResource | null = null;
      let bookReadError = "";
      try {
        updatedBook = await readResource<BookResource>(response.book);
        if (!isCurrent()) return;
        if (updatedBook.id !== bookId) throw new Error("清除后返回的书籍与当前操作不匹配。");
      } catch (error) {
        if (!isCurrent()) return;
        const responseReadError = errorText(error);
        try {
          updatedBook = await readResource<BookResource>(await getBook(bookId));
          if (!isCurrent()) return;
          if (updatedBook.id !== bookId) throw new Error("重读后返回的书籍与当前操作不匹配。");
        } catch (reloadError) {
          if (!isCurrent()) return;
          bookReadError = `${responseReadError}；重读失败：${errorText(reloadError)}`;
        }
      }
      const currentBook = openedBook.value;
      if (currentBook?.id === bookId) {
        openedBook.value = updatedBook ?? withoutChapterResources(currentBook);
      }
      if (bookReadError) {
        notify(`《${title}》的章节缓存已清除，但详情未能重新载入：${bookReadError}。`, "error");
      } else {
        const summary = response.clearedCount > 0 ? `已清除 ${response.clearedCount} 章正文缓存，可以重新下载。` : "没有发现已缓存的章节正文。";
        notify(`《${title}》${summary}进度、目录、封面和书签已保留。${warning ? ` ${warning}` : ""}`);
      }
      if (!response.recoveryRequired && !recoveryRequired.value) await refreshChapterCacheUsage();
    } catch (error) {
      if (isCurrent()) {
        bookClearError.value = errorText(error);
        notify(`清除《${title}》的章节缓存失败：${errorText(error)}`, "error");
      }
    } finally {
      if (isCurrent()) bookClearBusy.value = false;
    }
  }

  function resetForRestore(): void {
    chapterCacheUsage.value = null;
    chapterCacheUsageBusy.value = false;
    chapterCacheUsageError.value = "";
    pendingShelfClear.value = false;
    shelfClearBusy.value = false;
    shelfClearResult.value = null;
    pendingBookClear.value = null;
    bookClearBusy.value = false;
    bookClearError.value = "";
  }

  return { resetForRestore, lockChapterCacheRecovery, prepareReaderForChapterCacheClear, refreshChapterCacheUsage, formatChapterCacheBytes, clearShelfOnlineChapterCaches, clearOpenedBookChapterCache };
}
