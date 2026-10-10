import { command } from "./ipc";
import type { ResourceDescriptor, Bookmark, ReadingHistoryResource, ReadingStatisticsResource, DictionaryLanguage, DictionaryLookupResult } from "./types";

export const listBookmarks = () => command<ResourceDescriptor>("list_bookmarks");

export const upsertBookmark = (input: Omit<Bookmark, "id" | "createdAtMs" | "updatedAtMs"> & { id?: string }) =>
  command<ResourceDescriptor>("upsert_bookmark", { input });

export const deleteBookmark = (bookmarkId: string) =>
  command<ResourceDescriptor>("delete_bookmark", { bookmarkId });

export const readingHistoryResource = () => command<ResourceDescriptor>("reading_history_resource");

export const getReadingStatistics = (fromMs: number, toMs: number) =>
  command<ReadingStatisticsResource>("get_reading_statistics", { fromMs, toMs });

export const recordReadingSession = (session: ReadingHistoryResource["sessions"][number]) =>
  command<ResourceDescriptor>("record_reading_session", { session });

export const deleteReadingHistoryForBook = (bookId: string) =>
  command<ResourceDescriptor>("delete_reading_history_for_book", { bookId });

export const clearReadingHistory = () => command<ResourceDescriptor>("clear_reading_history");

export const lookupDictionary = (word: string, language: DictionaryLanguage) =>
  command<DictionaryLookupResult>("lookup_dictionary", { word, language });
