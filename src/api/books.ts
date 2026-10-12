import { command } from "./ipc";
import type { ResourceDescriptor, BookMetadataUpdate, BookCoverPickerResponse, ResetBookProgressResponse, ClearBookChapterCacheResponse, ChapterCacheUsage, ReadingProgress, RefreshBookInfoResponse, LocalBookImportResponse, LocalBookImportOptions, PendingExternalFile } from "./types";

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

export const prepareSearchResultBook = (resultId: string) =>
  command<{ book: ResourceDescriptor }>("prepare_search_result_book", { resultId });


export const saveBook = (book: BookMetadataUpdate, requestId: number) =>
  command<{ requestId: number; resource: ResourceDescriptor }>("save_book", { book, requestId });

export const refreshBookInfo = (bookId: string) =>
  command<RefreshBookInfoResponse>("refresh_book_info", { bookId });

export const pickBookCover = (bookId: string) =>
  command<BookCoverPickerResponse>("pick_book_cover", { bookId });

export const discardBookCoverAsset = (bookId: string, assetId: string) =>
  command<{ removed: boolean; inUse: boolean }>("discard_book_cover_asset", { bookId, assetId });

export const resetBookProgress = (bookId: string) =>
  command<ResetBookProgressResponse>("reset_book_progress", { bookId });

export const clearBookChapterCache = (bookId: string) =>
  command<ClearBookChapterCacheResponse>("clear_book_chapter_cache", { bookId });

export const getChapterCacheUsage = () => command<ChapterCacheUsage>("get_chapter_cache_usage");


/** Rust publishes progress and prepared resources through book-updated. */
export const notifyReading = (bookId: string, progress: ReadingProgress, refreshMedia = false) =>
  command<void>("notify_reading", { bookId, progress, refreshMedia });
