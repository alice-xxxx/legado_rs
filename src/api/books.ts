import { command } from "./ipc";
import type { ResourceDescriptor, BookDisplayMetadataPatch, BookCoverPickerResponse, BookDisplayMetadataUpdateResponse, RemoveBookResponse, ResetBookProgressResponse, ClearBookChapterCacheResponse, ChapterCacheUsage, ReadingProgress, BookMutationResponse, RefreshBookInfoResponse, LocalBookImportResponse, LocalBookImportOptions, PendingExternalFile } from "./types";

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

export const addBook = (resultId: string) =>
  command<BookMutationResponse>("add_book", { resultId });

export const prepareSearchResultBook = (resultId: string) =>
  command<{ book: ResourceDescriptor }>("prepare_search_result_book", { resultId });

export const getBook = (bookId: string) => command<ResourceDescriptor>("get_book", { bookId });

export const refreshBookInfo = (bookId: string) =>
  command<RefreshBookInfoResponse>("refresh_book_info", { bookId });

export const updateBookDisplayMetadata = (bookId: string, patch: BookDisplayMetadataPatch) =>
  command<BookDisplayMetadataUpdateResponse>("update_book_display_metadata", { bookId, patch });

export const pickBookCover = (bookId: string) =>
  command<BookCoverPickerResponse>("pick_book_cover", { bookId });

export const discardBookCoverAsset = (bookId: string, assetId: string) =>
  command<{ removed: boolean; inUse: boolean }>("discard_book_cover_asset", { bookId, assetId });

export const prepareChapters = (bookId: string, fromIndex: number, count: number) =>
  command<{ book: ResourceDescriptor; prepared: number }>("prepare_chapters", {
    bookId,
    fromIndex,
    count,
  });

export const refreshChapterContent = (bookId: string, chapterId: string) =>
  command<{
    commitState: "committed" | "notCommitted" | "indeterminate";
    recoveryRequired: boolean;
    warning?: string | null;
    error?: string | null;
    book?: ResourceDescriptor;
    chapterId?: string;
    fromIndex?: number;
    prepared?: number;
  }>("refresh_chapter_content", { bookId, chapterId });

export const removeBook = (bookId: string) =>
  command<RemoveBookResponse>("remove_book", { bookId });

export const resetBookProgress = (bookId: string) =>
  command<ResetBookProgressResponse>("reset_book_progress", { bookId });

export const clearBookChapterCache = (bookId: string) =>
  command<ClearBookChapterCacheResponse>("clear_book_chapter_cache", { bookId });

export const getChapterCacheUsage = () => command<ChapterCacheUsage>("get_chapter_cache_usage");

export const saveProgress = (bookId: string, progress: ReadingProgress) =>
  command<ResourceDescriptor>("save_progress", { bookId, progress });
