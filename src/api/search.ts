import { command } from "./ipc";
import type { ResourceDescriptor, SearchResponse, SourceDebugResponse, ChangeChapterSourceResponse, BookSourceMutationResponse, TaskResponse } from "./types";

export const getSearchHistory = () => command<ResourceDescriptor>("get_search_history");

export const deleteSearchHistory = (query: string) =>
  command<ResourceDescriptor>("delete_search_history", { query });

export const clearSearchHistory = () => command<ResourceDescriptor>("clear_search_history");

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
  command<TaskResponse>("search_book_source_candidates", {
    bookId,
    sourceIds,
    ...(keyword === undefined ? {} : { keyword }),
    page,
  });

export const changeBookSource = (bookId: string, resultId: string, confirmMissingAuthor = false) =>
  command<BookSourceMutationResponse>("change_book_source", { bookId, resultId, confirmMissingAuthor });

export const changeChapterSource = (bookId: string, chapterId: string, resultId: string, confirmMissingAuthor = false) =>
  command<ChangeChapterSourceResponse>("change_chapter_source", { bookId, chapterId, resultId, confirmMissingAuthor });
