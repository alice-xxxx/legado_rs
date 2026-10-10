import type { Ref } from "vue";
import { getBook, removeBook, updateBookDisplayMetadata } from "../../api/books";
import { readResource } from "../../api/resources";
import { type BookDisplayMetadataPatch, type BookResource, type ResourceDescriptor, type ShelfResource } from "../../api/types";

type PendingBookRemoval = { id: string; title: string };

export interface BookMetadataActionsOptions {
  openedBook: Ref<BookResource | null>;
  readingBook: Ref<BookResource | null>;
  shelf: Ref<ShelfResource>;
  editorBookId: Ref<string | null>;
  editorError: Ref<string>;
  saveBusy: Ref<boolean>;
  pendingRemoval: Ref<PendingBookRemoval | null>;
  removalBusy: Ref<boolean>;
  globalError: Ref<string>;
  recoveryRequired: Ref<boolean>;
  recoveryTitle: Ref<string>;
  recoveryMessage: Ref<string>;
  refreshShelfFromDescriptor: (descriptor: ResourceDescriptor) => Promise<void>;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理书籍展示信息编辑、资源重载和从书架移除。 */
export function useBookMetadataActions(options: BookMetadataActionsOptions) {
  const {
    openedBook,
    readingBook,
    shelf,
    editorBookId,
    editorError,
    saveBusy,
    pendingRemoval,
    removalBusy,
    globalError,
    recoveryRequired,
    recoveryTitle,
    recoveryMessage,
    refreshShelfFromDescriptor,
    getRestoreRevision,
    notify,
    errorText,
  } = options;
  let bookRefreshRevision = 0;

  function openBookDisplayEditor(): void {
    const book = openedBook.value;
    if (!book || saveBusy.value) return;
    editorBookId.value = book.id;
    editorError.value = "";
  }

  function closeBookDisplayEditor(): void {
    if (saveBusy.value) return;
    editorBookId.value = null;
    editorError.value = "";
  }

  async function saveBookDisplayMetadata(patch: BookDisplayMetadataPatch): Promise<void> {
    const bookAtStart = openedBook.value;
    const bookId = editorBookId.value;
    if (!bookAtStart || !bookId || bookAtStart.id !== bookId || saveBusy.value || recoveryRequired.value) return;

    const restoreRevision = getRestoreRevision();
    saveBusy.value = true;
    editorError.value = "";
    try {
      const response = await updateBookDisplayMetadata(bookId, patch);
      if (getRestoreRevision() !== restoreRevision) return;
      const warning = response.warning ?? "";
      const markRecoveryRequired = (title: string, message: string): void => {
        recoveryRequired.value = true;
        recoveryTitle.value = title;
        recoveryMessage.value = message;
      };

      if (response.commitState === "notCommitted") {
        const message = warning || "显示信息更新未提交。";
        if (response.recoveryRequired) {
          const restartMessage = `《${bookAtStart.title}》的显示信息未提交，但回滚或恢复尚未完成。请重启应用后再继续。${warning ? ` ${warning}` : ""}`;
          markRecoveryRequired("需要重启应用完成恢复", restartMessage);
          editorError.value = restartMessage;
          notify("显示信息未提交；恢复待完成，请重启应用。", "error");
        } else {
          editorError.value = message;
        }
        return;
      }

      if (response.commitState === "indeterminate") {
        const message = `《${bookAtStart.title}》的显示信息提交状态未知。为避免重复写入，请重启应用检查书籍状态。${warning ? ` ${warning}` : ""}`;
        markRecoveryRequired("显示信息提交状态未知", message);
        editorError.value = message;
        notify("显示信息提交状态未知；已暂停书架修改，请重启应用检查。", "error");
        return;
      }

      if (response.commitState !== "committed") {
        const message = `《${bookAtStart.title}》的显示信息提交状态未知。请重启应用检查后再继续。`;
        markRecoveryRequired("显示信息提交状态未知", message);
        editorError.value = message;
        notify("显示信息提交状态未知；已暂停书架修改，请重启应用检查。", "error");
        return;
      }

      if (response.recoveryRequired) {
        markRecoveryRequired(
          "需要重启应用完成恢复",
          `《${bookAtStart.title}》的显示信息已保存，但清理或恢复待完成。请重启应用。${warning ? ` ${warning}` : ""}`,
        );
      }

      let updatedBook: BookResource | null = null;
      let bookReadError = "";
      try {
        updatedBook = await readResource<BookResource>(response.book);
        if (updatedBook.id !== bookId) throw new Error("保存后返回的书籍与当前编辑项不匹配。");
      } catch (error) {
        const responseReadError = errorText(error);
        try {
          updatedBook = await readResource<BookResource>(await getBook(bookId));
          if (updatedBook.id !== bookId) throw new Error("重读后返回的书籍与当前编辑项不匹配。");
        } catch (reloadError) {
          bookReadError = `${responseReadError}；重读失败：${errorText(reloadError)}`;
        }
      }

      let shelfReadError = "";
      try {
        await refreshShelfFromDescriptor(response.shelf);
      } catch (error) {
        shelfReadError = errorText(error);
      }

      // 恢复期间旧请求不能覆盖恢复后的详情；同一本书被并发更新时重读最新版本。
      let mayApplyToCurrentDetail = getRestoreRevision() === restoreRevision && openedBook.value?.id === bookId;
      if (updatedBook && mayApplyToCurrentDetail) {
        const current = openedBook.value;
        if (current && current !== bookAtStart) {
          try {
            const latest = await readResource<BookResource>(await getBook(bookId));
            if (latest.id !== bookId) throw new Error("重读后返回的书籍与当前编辑项不匹配。");
            updatedBook = latest;
          } catch (error) {
            bookReadError = `显示信息已保存，但并发更新后重读失败：${errorText(error)}`;
            updatedBook = null;
          }
          mayApplyToCurrentDetail = getRestoreRevision() === restoreRevision && openedBook.value === current;
        }
        if (mayApplyToCurrentDetail && updatedBook) openedBook.value = updatedBook;
      }

      if (editorBookId.value === bookId) editorBookId.value = null;
      if (bookReadError || shelfReadError) {
        const details = [bookReadError && `详情未能重新载入：${bookReadError}`, shelfReadError && `书架未能更新：${shelfReadError}`].filter(Boolean).join("；");
        notify(`显示信息已保存，但${details}。重新打开详情可再次读取。`, "error");
      } else if (response.recoveryRequired) {
        notify("显示信息已保存，但清理或恢复待完成；请重启应用。", "error");
      } else {
        notify("书籍显示信息已保存。", "success");
      }
    } catch (error) {
      if (editorBookId.value === bookId) {
        editorError.value = errorText(error);
      } else {
        notify(`保存书籍显示信息失败：${errorText(error)}`, "error");
      }
    } finally {
      saveBusy.value = false;
    }
  }

  async function refreshOpenedBook(descriptor?: ResourceDescriptor): Promise<void> {
    const currentReader = readingBook.value;
    const currentDetail = openedBook.value;
    const id = currentReader?.id ?? currentDetail?.id;
    if (!id && !descriptor) return;
    const restoreRevision = getRestoreRevision();
    const refreshRevision = ++bookRefreshRevision;
    const resource = descriptor ?? await getBook(id!);
    const book = await readResource<BookResource>(resource);
    if (restoreRevision !== getRestoreRevision() || refreshRevision !== bookRefreshRevision) return;
    // An event-triggered read must not replace a newer chapter, progress, or
    // book detail loaded while its immutable resource was in flight.
    if (currentReader?.id === book.id && readingBook.value === currentReader) readingBook.value = book;
    if (currentDetail?.id === book.id && openedBook.value === currentDetail) openedBook.value = book;
  }

  async function removeBookFromShelf(): Promise<void> {
    if (!pendingRemoval.value || removalBusy.value || recoveryRequired.value) return;
    const restoreRevision = getRestoreRevision();
    const { id, title } = pendingRemoval.value;
    globalError.value = "";
    removalBusy.value = true;
    try {
      const response = await removeBook(id);
      if (restoreRevision !== getRestoreRevision()) return;

      if (response.commitState !== "committed") {
        if (response.commitState === "notCommitted") {
          const errorMessage = response.error.trim() || "删除未提交。";
          if (!response.recoveryRequired) {
            notify(`《${title}》未移除，仍保留在书架。${errorMessage}`, "error");
          } else {
            pendingRemoval.value = null;
            recoveryRequired.value = true;
            recoveryTitle.value = "需要重启应用完成恢复";
            recoveryMessage.value = `《${title}》未移除，但回滚或恢复尚未完成。请重启应用后再继续。${errorMessage}`;
            notify("删除未提交；恢复待完成，请重启应用。", "error");
          }
        } else {
          const errorMessage = response.error.trim();
          pendingRemoval.value = null;
          recoveryRequired.value = true;
          recoveryTitle.value = "单本删除状态未知";
          recoveryMessage.value = `《${title}》的删除提交状态未知。请重启应用检查书架后再继续。${errorMessage ? ` ${errorMessage}` : ""}`;
          notify("删除状态未知，已暂停书架修改；请重启应用检查。", "error");
        }
        return;
      }

      pendingRemoval.value = null;
      if (openedBook.value?.id === id) openedBook.value = null;
      shelf.value = { ...shelf.value, books: shelf.value.books.filter((book) => book.id !== id) };
      const warning = response.warning?.trim() ?? "";
      if (response.recoveryRequired) {
        recoveryRequired.value = true;
        recoveryTitle.value = "需要重启应用完成恢复";
        recoveryMessage.value = `《${title}》已从书架移除，但清理或恢复仍待完成。请重启应用。${warning ? ` ${warning}` : ""}`;
      }
      try {
        await refreshShelfFromDescriptor(response.shelf);
      } catch (error) {
        if (restoreRevision === getRestoreRevision()) {
          globalError.value = `《${title}》已从书架移除，但书架显示刷新失败：${errorText(error)}`;
        }
      }
      if (restoreRevision !== getRestoreRevision()) return;
      if (response.recoveryRequired) notify("书籍已移除，但恢复或清理待完成；请重启应用。", "error");
      else if (globalError.value) notify(globalError.value, "error");
      else notify(`《${title}》已从书架移除。${warning ? ` ${warning}` : ""}`);
    } catch (error) {
      if (restoreRevision !== getRestoreRevision()) return;
      const commandError = errorText(error);
      globalError.value = "";
      pendingRemoval.value = null;
      recoveryRequired.value = true;
      recoveryTitle.value = "单本删除结果未确认";
      recoveryMessage.value = `没有收到《${title}》的结构化删除结果，无法确认是否已从书架移除。请重启应用检查书架后再尝试。应用返回信息：${commandError}`;
      notify("删除结果未知，请重启应用检查书架后再继续。", "error");
    } finally {
      removalBusy.value = false;
    }
  }

  return { openBookDisplayEditor, closeBookDisplayEditor, saveBookDisplayMetadata, refreshOpenedBook, removeBookFromShelf };
}
