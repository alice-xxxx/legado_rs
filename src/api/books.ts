import { command } from "./ipc";
import { modifyDatabase, type DatabaseChange } from "./feDb";
import { storedAssetUrl } from "./database";
import type { BookMetadataUpdate, ChapterResource, BookCoverPickerResponse, ClearBookChapterCacheResponse, ChapterCacheUsage, ReadingProgress, LocalBookImportResponse, LocalBookImportOptions, PendingExternalFile } from "./types";

export type LocalImportFormat = "txt" | "epub" | "cbz" | "pdf";
export const importBookFromPicker = (options: LocalBookImportOptions = {}, format?: LocalImportFormat) =>
  command<LocalBookImportResponse>("import_book_from_picker", { options, format });

export const listPendingExternalFiles = () =>
  command<{ files: PendingExternalFile[] }>("list_pending_external_files");

export const importExternalFile = (token: string, options: LocalBookImportOptions = {}) =>
  command<LocalBookImportResponse>("import_external_file", { token, options });

export const discardExternalFile = (token: string) =>
  command<{ discarded: boolean }>("discard_external_file", { token });

export const importProtectedPdf = (importToken: string, password: string) =>
  command<LocalBookImportResponse>("import_protected_pdf", { importToken, password });

export const cancelPendingPdfImport = (importToken: string) =>
  command<{ cancelled: boolean }>("cancel_pending_pdf_import", { importToken });

export const saveBook = (book: BookMetadataUpdate) => modifyDatabase([{
  action: "update", table: "books", filter: { id: book.id }, values: {
    title_override: book.displayOverrides?.title?.trim() || null, author_override: book.displayOverrides?.author?.trim() || null,
    intro_override: book.displayOverrides?.intro?.trim() || null,
    cover_override: book.displayOverrides?.coverSrc ? storedAssetUrl(book.displayOverrides.coverSrc) : null,
  },
}]);

/** 只保存 Vue 选择的章节缓存地址，不请求正文。 */
export const saveChapter = (bookId: string, chapter: ChapterResource) => {
  const filter = { book_id: bookId, chapter_id: chapter.id };
  // 保留章节身份和引擎字段，只替换 Vue 选定的缓存地址。
  const changes: DatabaseChange[] = [{ action: "update", table: "chapters", filter: { book_id: bookId, id: chapter.id },
    values: { resource_url: chapter.resource?.resourceId ?? null } }, { action: "delete", table: "chapter_pages", filter }];
  for (const [position, page] of (chapter.pages ?? []).entries()) changes.push({ action: "insert", table: "chapter_pages",
    values: { ...filter, position, id: page.id, url: storedAssetUrl(page.url ?? page.src) } });
  return modifyDatabase(changes);
};

export const pickBookCover = (bookId: string) =>
  command<BookCoverPickerResponse>("pick_book_cover", { bookId });

export const discardBookCoverAsset = (bookId: string, assetId: string) =>
  command<{ removed: boolean; inUse: boolean }>("discard_book_cover_asset", { bookId, assetId });

export const resetBookProgress = (bookId: string) =>
  modifyDatabase([{ action: "delete", table: "reading_progress", filter: { book_id: bookId } }]);

export const clearBookChapterCache = (bookId: string) =>
  command<ClearBookChapterCacheResponse>("clear_book_chapter_cache", { bookId });

export const getChapterCacheUsage = () => command<ChapterCacheUsage>("get_chapter_cache_usage");


/** Progress arrives as a small delta; prepared resources use book-updated. */
export const notifyReading = (bookId: string, progress: ReadingProgress, refreshMedia = false, prepareCurrent = false) =>
  command<void>("notify_reading", { bookId, progress, refreshMedia, prepareCurrent });
