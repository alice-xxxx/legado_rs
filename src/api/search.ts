import { command } from "./ipc";
import { Channel } from "@tauri-apps/api/core";
import { taskCommand } from "./tasks";
import type { SearchResponse, SearchResource, SearchBatch, SearchType, SourceDebugResponse, ChangeChapterSourceResponse, BookSourceMutationResponse } from "./types";

export const searchBooks = (sourceIds: string[], keyword: string, page = 1) =>
  command<SearchResponse>("search_books", { sourceIds, keyword, page });

export const debugSearchResult = (resultId?: string, chapterIndex?: number) =>
  command<SourceDebugResponse>("debug_search_result", {
    ...(resultId === undefined ? {} : { resultId }),
    ...(chapterIndex === undefined ? {} : { chapterIndex }),
  });

export const clearSourceDebugPreview = (resourceId: string) =>
  command<SourceDebugResponse>("debug_search_result", { cleanupResourceId: resourceId });

export function startSearch(sourceType: SearchType, sourceIds: string[], keyword: string,
  page: number, precisionSearch: boolean, onResults: (batch: SearchBatch) => void) {
  const channel = new Channel<SearchBatch>();
  channel.onmessage = onResults;
  return command<{ taskId: string }>("start_search", { sourceType, sourceIds, keyword, page, precisionSearch, onResults: channel });
}

export const searchBookSourceCandidates = (bookId: string, sourceIds: string[], keyword?: string, page = 1) =>
  taskCommand("search_book_source_candidates", {
    bookId,
    sourceIds,
    ...(keyword === undefined ? {} : { keyword }),
    page,
  });

export const changeBookSource = (bookId: string, sourceBookId: string, confirmMissingAuthor = false) =>
  command<BookSourceMutationResponse>("change_book_source", { bookId, sourceBookId, confirmMissingAuthor });

export const changeChapterSource = (bookId: string, chapterId: string, resultId: string, confirmMissingAuthor = false, alternativeIndex?: number, alternativeSourceId?: string) =>
  command<ChangeChapterSourceResponse>("change_chapter_source", { bookId, chapterId, resultId, confirmMissingAuthor, alternativeIndex, alternativeSourceId });

export const getSearchTaskResults = (taskId: string) =>
  command<SearchResource>("get_search_task_results", { taskId });

export const releaseSearchResults = (taskId: string) => command<void>("release_search_results", { taskId });

// Discovery cancellation does not wait for the task-center resource to reload.
export const stopSearch = (taskId: string) => command("cancel_task", { taskId });
