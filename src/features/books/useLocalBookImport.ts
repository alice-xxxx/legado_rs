import { computed, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { backupBusy } from "../settings/backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { importBook, type BookImportProgress, type LocalImportFormat } from "../../api/books";

/** 导入只等待命令完成；处理进度和完成的书籍 ID 由 Rust 直接 emit。 */
export function useLocalBookImport(options: {
  notify: (message: string, kind?: "success" | "error") => void;
}) {
  const importingFiles = ref<Record<string, BookImportProgress>>({});
  const selecting = ref(false);
  const localBookImportBusy = computed(() => selecting.value || Object.keys(importingFiles.value).length > 0);
  let listening: Promise<UnlistenFn> | undefined;
  let disposed = false;

  function receive(progress: BookImportProgress): void {
    if (disposed) return;
    if (progress.stage !== "done" && progress.stage !== "error") {
      importingFiles.value[progress.file] = progress;
      return;
    }
    delete importingFiles.value[progress.file];
    if (progress.bookId) options.notify(`${progress.file} 已导入书架。`);
  }

  function setupImports(): Promise<UnlistenFn> {
    return listening ??= listen<BookImportProgress>("book-import", event => {
      receive(event.payload);
    });
  }

  async function importLocalBook(format?: LocalImportFormat): Promise<void> {
    if (localBookImportBusy.value || backupBusy.value || shelfBatchRecoveryRequired.value) return;
    selecting.value = true;
    try {
      await setupImports();
      await importBook(format);
    } finally { selecting.value = false; }
  }

  function dispose(): void {
    disposed = true;
    void listening?.then(unlisten => unlisten());
  }

  return { localBookImportBusy, importingFiles, importLocalBook, setupImports, dispose };
}
