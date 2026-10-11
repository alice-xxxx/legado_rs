import { command } from "./ipc";
import { taskCommand } from "./tasks";
import type { SearchResponse, SourceDebugResponse, ChangeChapterSourceResponse, BookSourceMutationResponse } from "./types";

export const searchBooks = (sourceIds: string[], keyword: string, page = 1) =>
  command<SearchResponse>("search_books", { sourceIds, keyword, page });

export const debugSearchResult = (resultId?: string, chapterIndex?: number) =>
  command<SourceDebugResponse>("debug_search_result", {
    ...(resultId === undefined ? {} : { resultId }),
    ...(chapterIndex === undefined ? {} : { chapterIndex }),
  });

export const clearSourceDebugPreview = (resourceId: string) =>
  command<SourceDebugResponse>("debug_search_result", { cleanupResourceId: resourceId });

export const startSearch = (sourceIds: string[], keyword: string, page = 1) =>
  command<{ taskId: string }>("start_search", { sourceIds, keyword, page });

export const searchBookSourceCandidates = (bookId: string, sourceIds: string[], keyword?: string, page = 1) =>
  taskCommand("search_book_source_candidates", {
    bookId,
    sourceIds,
    ...(keyword === undefined ? {} : { keyword }),
    page,
  });

export const changeBookSource = (bookId: string, sourceBookId: string, confirmMissingAuthor = false) =>
  command<BookSourceMutationResponse>("change_book_source", { bookId, sourceBookId, confirmMissingAuthor });

export const changeChapterSource = (bookId: string, chapterId: string, resultId: string, confirmMissingAuthor = false) =>
  command<ChangeChapterSourceResponse>("change_chapter_source", { bookId, chapterId, resultId, confirmMissingAuthor });
