import { command } from "./ipc";
import { modifyDatabase } from "./feDb";
import { storedAssetUrl } from "./database";
import type { BookMetadataUpdate, ChapterResource, BookCoverPickerResponse, ClearBookChapterCacheResponse, ChapterCacheUsage, ReadingProgress, LocalBookImportOptions } from "./types";

export type LocalImportFormat = "txt" | "epub" | "cbz" | "pdf";
export const importBook = (format?: LocalImportFormat, options: LocalBookImportOptions = {}) =>
  command<void>("import_book", { format, options });

export interface BookImportProgress {
  file: string;
  stage: "read" | "copy" | "parse" | "cache" | "save" | "done" | "error";
  completed?: number;
  total?: number;
  bookId?: string;
  error?: string;
}

export const saveBook = (book: BookMetadataUpdate) => modifyDatabase([{
  operation: "saveBook", bookId: book.id, changes: {
    title: book.displayOverrides?.title?.trim() || null,
    author: book.displayOverrides?.author?.trim() || null,
    intro: book.displayOverrides?.intro?.trim() || null,
    cover: book.displayOverrides?.coverSrc ? storedAssetUrl(book.displayOverrides.coverSrc) : null,
  },
}], { wait: true });

/** 提交章节缓存引用，关联维护由数据库完成。 */
export const saveChapter = (bookId: string, chapter: ChapterResource) => modifyDatabase([{
  operation: "useChapterContent", bookId, chapterId: chapter.id,
  contentId: chapter.contentId ?? "",
}], { wait: true });

export const pickBookCover = (bookId: string) =>
  command<BookCoverPickerResponse>("pick_book_cover", { bookId });

export const discardBookCoverAsset = (bookId: string, assetId: string) =>
  command<{ removed: boolean; inUse: boolean }>("discard_book_cover_asset", { bookId, assetId });

export const resetBookProgress = (bookId: string) =>
  modifyDatabase([{ operation: "resetProgress", bookId }], { wait: true });

export const clearBookChapterCache = (bookId: string) =>
  command<ClearBookChapterCacheResponse>("clear_book_chapter_cache", { bookId });

export const getChapterCacheUsage = () => command<ChapterCacheUsage>("get_chapter_cache_usage");


/** Progress arrives as a small delta; prepared resources use book-updated. */
export async function notifyReading(bookId: string, progress: ReadingProgress, prepareCurrent = false): Promise<void> {
  await modifyDatabase([{ operation: "saveProgress", bookId, progress }], { wait: true, prepareCurrent });
}
