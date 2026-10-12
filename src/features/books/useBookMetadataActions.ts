import type { Ref } from "vue";
import { shelf as shelfPositions } from "../shelf/shelfStore";
import { type BookDisplayMetadataPatch, type ResourceDescriptor } from "../../api/types";
import { openedBook, refreshOpenedBook as refreshBookResource } from "./bookDetailState";

type PendingBookRemoval = { id: string; title: string };

export interface BookMetadataActionsOptions {
  editorBookId: Ref<string | null>;
  pendingRemoval: Ref<PendingBookRemoval | null>;
  recoveryRequired: Ref<boolean>;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
}

/** 管理书籍展示信息编辑、资源重载和从书架移除。 */
export function useBookMetadataActions(options: BookMetadataActionsOptions) {
  const {
    editorBookId,
    pendingRemoval,
    recoveryRequired,
    getRestoreRevision,
    notify,
  } = options;

  function openBookDisplayEditor(): void {
    const book = openedBook.value;
    if (!book) return;
    editorBookId.value = book.id;
  }

  function closeBookDisplayEditor(): void {
    editorBookId.value = null;
  }

  function saveBookDisplayMetadata(patch: BookDisplayMetadataPatch): void {
    const bookAtStart = openedBook.value;
    const bookId = editorBookId.value;
    if (!bookAtStart || !bookId || bookAtStart.id !== bookId || recoveryRequired.value) return;

    const overrides = { ...(bookAtStart.displayOverrides ?? {}) };
    for (const field of ["title", "author", "intro", "coverSrc"] as const) {
      if (!(field in patch)) continue;
      if (patch[field] === null) delete overrides[field];
      else if (patch[field] !== undefined) overrides[field] = patch[field];
    }
    bookAtStart.displayOverrides = Object.keys(overrides).length ? overrides : undefined;
    editorBookId.value = null;
  }

  async function refreshOpenedBook(descriptor?: ResourceDescriptor): Promise<void> {
    const id = openedBook.value?.id;
    if (!id && !descriptor) return;
    const restoreRevision = getRestoreRevision();
    const resource = descriptor ?? id!;
    if (restoreRevision !== getRestoreRevision()) return;
    await refreshBookResource(resource);
  }

  function removeBookFromShelf(): void {
    if (!pendingRemoval.value || recoveryRequired.value) return;
    const { id, title } = pendingRemoval.value;
    shelfPositions.value.books = shelfPositions.value.books.filter(book => book.id !== id);
    pendingRemoval.value = null;
    notify(`《${title}》已从书架移除，缓存和阅读记录保留。`);
  }

  return { openBookDisplayEditor, closeBookDisplayEditor, saveBookDisplayMetadata, refreshOpenedBook, removeBookFromShelf };
}
