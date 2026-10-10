import { ref, type Ref } from "vue";
import { cancelPendingPdfImport, importBookFromPicker, importProtectedPdf, type LocalImportFormat } from "../../api/books";
import { readResource } from "../../api/resources";
import { type BookResource, type LocalBookImportResponse, type ShelfResource } from "../../api/types";

interface LocalBookImportOptions {
  openedBook: Ref<BookResource | null>;
  shelf: Ref<ShelfResource>;
  externalFileRequestBusy: Readonly<Ref<boolean>>;
  backupBusy: Readonly<Ref<unknown>>;
  recoveryRequired: Readonly<Ref<boolean>>;
  getRestoreRevision: () => number;
  pdfImportPasswords: Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理本地文件导入、受保护 PDF 的密码提示及阅读器临时密码。 */
export function useLocalBookImport(options: LocalBookImportOptions) {
  const localBookImportBusy = ref(false);
  const pdfPasswordPromptOpen = ref(false);
  const pendingPdfImportToken = ref<string | null>(null);
  const pdfPasswordBusy = ref(false);
  const pdfPasswordError = ref("");

  function openPdfPasswordPrompt(importToken: string): void {
    pendingPdfImportToken.value = importToken;
    pdfPasswordError.value = "";
    pdfPasswordPromptOpen.value = true;
  }

  async function acceptLocalBookImport(response: LocalBookImportResponse, initialPdfPassword = ""): Promise<void> {
    if (!response.book || !response.shelf) throw new Error("导入完成后没有返回书籍资源。");
    const restoreRevision = options.getRestoreRevision();
    try {
      const [book, nextShelf] = await Promise.all([
        readResource<BookResource>(response.book),
        readResource<ShelfResource>(response.shelf),
      ]);
      if (restoreRevision !== options.getRestoreRevision() || options.recoveryRequired.value) return;

      if (initialPdfPassword) {
        const previous = options.pdfImportPasswords.get(book.id);
        if (previous) clearTimeout(previous.timer);
        const entry = {
          password: initialPdfPassword,
          timer: setTimeout(() => {
            if (options.pdfImportPasswords.get(book.id) === entry) options.pdfImportPasswords.delete(book.id);
          }, 5 * 60 * 1000),
        };
        options.pdfImportPasswords.set(book.id, entry);
      }
      options.openedBook.value = book;
      options.shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
      options.notify(`《${book.title}》已加入书架。`);
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`文件已导入，但书架刷新失败：${options.errorText(error)}`, "error");
      }
    }
  }

  async function importLocalBook(format?: LocalImportFormat): Promise<void> {
    if (localBookImportBusy.value || options.externalFileRequestBusy.value || options.backupBusy.value
      || options.recoveryRequired.value || pdfPasswordPromptOpen.value) return;
    const restoreRevision = options.getRestoreRevision();
    localBookImportBusy.value = true;
    try {
      const response = await importBookFromPicker({}, format);
      if (restoreRevision !== options.getRestoreRevision()) {
        if (response.importToken) void cancelPendingPdfImport(response.importToken).catch(() => {});
        return;
      }
      if (response.cancelled) return;
      if (response.passwordRequired) {
        if (!response.importToken) throw new Error("无法继续打开加密文件，请重新选择文件。");
        openPdfPasswordPrompt(response.importToken);
        return;
      }
      await acceptLocalBookImport(response);
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`导入本地书失败：${options.errorText(error)}`, "error");
      }
    } finally {
      localBookImportBusy.value = false;
    }
  }

  async function submitPdfPassword(password: string): Promise<void> {
    const importToken = pendingPdfImportToken.value;
    if (!importToken || !password || pdfPasswordBusy.value) return;
    const restoreRevision = options.getRestoreRevision();
    pdfPasswordBusy.value = true;
    pdfPasswordError.value = "";
    try {
      const response = await importProtectedPdf(importToken, password);
      if (restoreRevision !== options.getRestoreRevision() || pendingPdfImportToken.value !== importToken) return;
      pendingPdfImportToken.value = null;
      pdfPasswordPromptOpen.value = false;
      await acceptLocalBookImport(response, password);
    } catch (error) {
      if (restoreRevision !== options.getRestoreRevision() || pendingPdfImportToken.value !== importToken) return;
      const message = options.errorText(error);
      pdfPasswordError.value = /(?:incorrect.*password|invalid password|password.*incorrect)/i.test(message)
        ? "密码不正确，请再试一次。"
        : /expired/i.test(message)
          ? "文件选择已过期，请重新选择文件。"
          : "无法打开文件，请检查密码后重试。";
    } finally {
      password = "";
      pdfPasswordBusy.value = false;
    }
  }

  async function cancelPdfPasswordPrompt(): Promise<void> {
    const importToken = pendingPdfImportToken.value;
    pendingPdfImportToken.value = null;
    pdfPasswordError.value = "";
    pdfPasswordPromptOpen.value = false;
    if (!importToken) return;
    try {
      await cancelPendingPdfImport(importToken);
    } catch (error) {
      options.notify(`取消导入失败：${options.errorText(error)}`, "error");
    }
  }

  // 组件卸载时释放尚未提交的导入令牌；错误由系统文件请求恢复流程处理。
  function cancelPendingPdfImportOnUnmount(): void {
    const importToken = pendingPdfImportToken.value;
    pendingPdfImportToken.value = null;
    if (importToken) void cancelPendingPdfImport(importToken).catch(() => {});
  }

  return {
    localBookImportBusy,
    pdfPasswordPromptOpen,
    pendingPdfImportToken,
    pdfPasswordBusy,
    pdfPasswordError,
    importLocalBook,
    openPdfPasswordPrompt,
    acceptLocalBookImport,
    submitPdfPassword,
    cancelPdfPasswordPrompt,
    cancelPendingPdfImportOnUnmount,
  };
}
