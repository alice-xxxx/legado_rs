import { computed, ref, type Ref } from "vue";
import { clearReadingHistory, deleteBookmark, deleteReadingHistoryForBook, getReadingStatistics, listBookmarks, readingHistoryResource, recordReadingSession, upsertBookmark } from "../../api/reading";
import { getBook } from "../../api/books";
import { readResource } from "../../api/resources";
import { type Bookmark, type BookmarksResource, type BookResource, type ReadingHistoryResource, type ReadingStatisticsResource, type ResourceDescriptor } from "../../api/types";

export type ReadingStatisticsPeriod = "today" | "7days" | "30days" | "all";

interface ReaderBookmarksAndHistoryOptions {
  readingBook: Ref<BookResource | null>;
  readingChapterIndex: Ref<number>;
  getReaderCurrentOffset: () => number;
  isBackupRestoreInProgress: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, requestedIndex?: number, requestedOffset?: number) => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理阅读书签、阅读会话与统计；持久化仍统一由 Rust 命令处理。 */
export function useReaderBookmarksAndHistory(options: ReaderBookmarksAndHistoryOptions) {
  const bookmarks = ref<Bookmark[]>([]);
  const readingHistory = ref<ReadingHistoryResource>({ schemaVersion: 1, sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 });
  const readingStatisticsPeriod = ref<ReadingStatisticsPeriod>("30days");
  const filteredReadingStatistics = ref<ReadingStatisticsResource | null>(null);
  const readingStatisticsBusy = ref(false);
  const readingStatisticsError = ref("");
  const bookmarkEditorOpen = ref(false);
  const bookmarkNote = ref("");
  const bookmarkEditingId = ref<string | null>(null);
  const bookmarkBusy = ref(false);
  const bookmarkDeleteBatchBusy = ref(false);
  const bookmarkDeleteBatchRequestId = ref(0);
  const bookmarkDeleteBatchResult = ref<{ requestId: number; deletedIds: string[]; failedIds: string[] } | null>(null);
  const readingSessionId = ref<string | null>(null);
  const readingSessionStartedAt = ref<number | null>(null);
  let readingStatisticsGeneration = 0;
  let bookmarkSnapshotRevision = 0;
  let historySnapshotRevision = 0;

  const readingStatisticsData = computed<ReadingStatisticsResource>(() => readingStatisticsPeriod.value === "all"
    ? readingHistory.value
    : filteredReadingStatistics.value ?? {
        books: [],
        days: [],
        totalDurationMs: 0,
        totalSessions: 0,
      });
  const currentBookmark = computed(() => bookmarks.value.find((bookmark) => bookmark.bookId === options.readingBook.value?.id
    && bookmark.chapterIndex === options.readingChapterIndex.value
    && bookmark.offset === options.getReaderCurrentOffset()) ?? null);

  async function applyBookmarksResource(descriptor: ResourceDescriptor): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const requestRevision = ++bookmarkSnapshotRevision;
    const document = await readResource<BookmarksResource>(descriptor);
    if (restoreRevision !== options.getRestoreRevision() || requestRevision !== bookmarkSnapshotRevision
      || options.isBackupRestoreInProgress()) return;
    bookmarks.value = Array.isArray(document.bookmarks) ? document.bookmarks : [];
  }

  async function applyReadingHistoryResource(descriptor: ResourceDescriptor): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const requestRevision = ++historySnapshotRevision;
    const document = await readResource<ReadingHistoryResource>(descriptor);
    if (restoreRevision !== options.getRestoreRevision() || requestRevision !== historySnapshotRevision
      || options.isBackupRestoreInProgress()) return;
    readingHistory.value = document;
    await refreshCurrentReadingStatistics();
  }

  async function refreshBookmarksAndReadingHistory(): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const bookmarkRevision = ++bookmarkSnapshotRevision;
    const historyRevision = ++historySnapshotRevision;
    const [bookmarksDescriptor, historyDescriptor] = await Promise.all([listBookmarks(), readingHistoryResource()]);
    const [nextBookmarks, nextHistory] = await Promise.all([
      readResource<BookmarksResource>(bookmarksDescriptor),
      readResource<ReadingHistoryResource>(historyDescriptor),
    ]);
    if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return;
    if (bookmarkRevision === bookmarkSnapshotRevision) bookmarks.value = Array.isArray(nextBookmarks.bookmarks) ? nextBookmarks.bookmarks : [];
    if (historyRevision !== historySnapshotRevision) return;
    readingHistory.value = {
      schemaVersion: nextHistory.schemaVersion ?? 1,
      sessions: Array.isArray(nextHistory.sessions) ? nextHistory.sessions : [],
      books: Array.isArray(nextHistory.books) ? nextHistory.books : [],
      days: Array.isArray(nextHistory.days) ? nextHistory.days : [],
      totalDurationMs: nextHistory.totalDurationMs ?? 0,
      totalSessions: nextHistory.totalSessions ?? 0,
    };
    await refreshCurrentReadingStatistics();
  }

  function readingStatisticsDateRange(period: Exclude<ReadingStatisticsPeriod, "all">, now = new Date()): { fromMs: number; toMs: number } {
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const to = new Date(today);
    to.setDate(to.getDate() + 1);
    const from = new Date(today);
    const days = period === "today" ? 1 : period === "7days" ? 7 : 30;
    from.setDate(from.getDate() - days + 1);
    return { fromMs: from.getTime(), toMs: to.getTime() };
  }

  async function refreshCurrentReadingStatistics(): Promise<void> {
    const generation = ++readingStatisticsGeneration;
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => generation === readingStatisticsGeneration
      && restoreRevision === options.getRestoreRevision() && !options.isBackupRestoreInProgress();
    const period = readingStatisticsPeriod.value;
    if (period === "all") {
      filteredReadingStatistics.value = null;
      readingStatisticsBusy.value = false;
      readingStatisticsError.value = "";
      return;
    }

    const { fromMs, toMs } = readingStatisticsDateRange(period);
    filteredReadingStatistics.value = null;
    readingStatisticsBusy.value = true;
    readingStatisticsError.value = "";
    try {
      const statistics = await getReadingStatistics(fromMs, toMs);
      if (isCurrent()) filteredReadingStatistics.value = statistics;
    } catch (error) {
      if (isCurrent()) readingStatisticsError.value = options.errorText(error);
    } finally {
      if (isCurrent()) readingStatisticsBusy.value = false;
    }
  }

  function changeReadingStatisticsPeriod(period: ReadingStatisticsPeriod): void {
    readingStatisticsPeriod.value = period;
    void refreshCurrentReadingStatistics();
  }

  async function continueReading(bookId: string): Promise<void> {
    try {
      const descriptor = await getBook(bookId);
      const book = await readResource<BookResource>(descriptor);
      await options.startReading(book);
    } catch (error) {
      options.notify(`打开阅读位置失败：${options.errorText(error)}`, "error");
    }
  }

  function toggleCurrentBookmark(): void {
    const existing = currentBookmark.value;
    if (existing) {
      void removeBookmark(existing.id);
    } else {
      bookmarkEditingId.value = null;
      bookmarkNote.value = "";
      bookmarkEditorOpen.value = true;
    }
  }

  function editCurrentBookmarkNote(): void {
    const bookmark = currentBookmark.value;
    if (!bookmark) return;
    bookmarkEditingId.value = bookmark.id;
    bookmarkNote.value = bookmark.note;
    bookmarkEditorOpen.value = true;
  }

  function closeBookmarkEditor(): void {
    bookmarkEditorOpen.value = false;
    bookmarkEditingId.value = null;
    bookmarkNote.value = "";
  }

  function beginReadingSession(): void {
    if (!options.readingBook.value || readingSessionId.value) return;
    readingSessionId.value = globalThis.crypto?.randomUUID?.() ?? `session-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    readingSessionStartedAt.value = Date.now();
  }

  async function finishReadingSession(): Promise<void> {
    if (options.isBackupRestoreInProgress()) return;
    const restoreRevision = options.getRestoreRevision();
    const historyRevision = ++historySnapshotRevision;
    const bookId = options.readingBook.value?.id;
    const sessionId = readingSessionId.value;
    const startedAt = readingSessionStartedAt.value;
    readingSessionId.value = null;
    readingSessionStartedAt.value = null;
    if (!bookId || !sessionId || startedAt == null) return;
    const endedAtMs = Date.now();
    try {
      const descriptor = await recordReadingSession({
        bookId,
        sessionId,
        durationMs: Math.max(0, endedAtMs - startedAt),
        endedAtMs,
        utcOffsetMinutes: -new Date(endedAtMs).getTimezoneOffset(),
      });
      const history = await readResource<ReadingHistoryResource>(descriptor);
      if (restoreRevision !== options.getRestoreRevision() || historyRevision !== historySnapshotRevision
        || options.isBackupRestoreInProgress()) return;
      readingHistory.value = history;
      await refreshCurrentReadingStatistics();
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`保存阅读记录失败：${options.errorText(error)}`, "error");
      }
    }
  }

  async function openBookmark(bookmark: Bookmark): Promise<void> {
    if (bookmark.orphaned) {
      options.notify(`“${bookmark.chapterTitle || `第 ${bookmark.chapterIndex + 1} 章`}”无法与新书源目录匹配，书签仍保留在列表中。`, "error");
      return;
    }
    try {
      const descriptor = await getBook(bookmark.bookId);
      const book = await readResource<BookResource>(descriptor);
      await options.startReading(book, bookmark.chapterIndex, bookmark.offset);
    } catch (error) {
      options.notify(`打开书签失败：${options.errorText(error)}`, "error");
    }
  }

  async function saveBookmark(): Promise<void> {
    const book = options.readingBook.value;
    if (!book || bookmarkBusy.value) return;
    const bookmarkToEdit = bookmarkEditingId.value
      ? bookmarks.value.find((bookmark) => bookmark.id === bookmarkEditingId.value)
      : null;
    if (bookmarkEditingId.value && !bookmarkToEdit) {
      closeBookmarkEditor();
      options.notify("这条书签已不存在，请重新打开书签列表。", "error");
      return;
    }
    if (options.isBackupRestoreInProgress()) return;
    bookmarkBusy.value = true;
    const restoreRevision = options.getRestoreRevision();
    try {
      const note = bookmarkNote.value.trim();
      const input = bookmarkToEdit
        ? {
            id: bookmarkToEdit.id,
            bookId: bookmarkToEdit.bookId,
            chapterIndex: bookmarkToEdit.chapterIndex,
            offset: bookmarkToEdit.offset,
            note,
          }
        : {
            bookId: book.id,
            chapterIndex: options.readingChapterIndex.value,
            offset: options.getReaderCurrentOffset(),
            note,
          };
      const descriptor = await upsertBookmark(input);
      const resource = await readResource<BookmarksResource>(descriptor);
      if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return;
      bookmarkSnapshotRevision += 1;
      bookmarks.value = resource.bookmarks ?? [];
      closeBookmarkEditor();
      options.notify(bookmarkToEdit ? "书签备注已更新。" : "书签已保存。");
    } catch (error) {
      options.notify(`${bookmarkToEdit ? "更新书签备注" : "保存书签"}失败：${options.errorText(error)}`, "error");
    } finally {
      bookmarkBusy.value = false;
    }
  }

  async function deleteBookmarkFromList(bookmarkId: string, showNotification: boolean): Promise<boolean> {
    if (options.isBackupRestoreInProgress()) return false;
    const restoreRevision = options.getRestoreRevision();
    try {
      const descriptor = await deleteBookmark(bookmarkId);
      const resource = await readResource<BookmarksResource>(descriptor);
      if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return false;
      bookmarkSnapshotRevision += 1;
      bookmarks.value = resource.bookmarks ?? [];
      if (showNotification) options.notify("书签已删除。");
      return true;
    } catch (error) {
      if (showNotification) options.notify(`删除书签失败：${options.errorText(error)}`, "error");
      return false;
    }
  }

  async function removeBookmark(bookmarkId: string): Promise<boolean> {
    if (bookmarkDeleteBatchBusy.value || bookmarkBusy.value) return false;
    bookmarkDeleteBatchBusy.value = true;
    try {
      return await deleteBookmarkFromList(bookmarkId, true);
    } finally {
      bookmarkDeleteBatchBusy.value = false;
    }
  }

  async function removeBookmarks(bookmarkIds: string[]): Promise<void> {
    if (bookmarkDeleteBatchBusy.value || bookmarkBusy.value || options.isBackupRestoreInProgress()) return;
    const restoreRevision = options.getRestoreRevision();
    const ids = [...new Set(bookmarkIds)].filter((id) => bookmarks.value.some((bookmark) => bookmark.id === id));
    if (!ids.length) return;
    bookmarkDeleteBatchBusy.value = true;
    bookmarkDeleteBatchResult.value = null;
    const deletedIds: string[] = [];
    const failedIds: string[] = [];
    try {
      for (const id of ids) {
        if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return;
        if (await deleteBookmarkFromList(id, false)) deletedIds.push(id);
        else failedIds.push(id);
      }
      bookmarkDeleteBatchRequestId.value += 1;
      bookmarkDeleteBatchResult.value = {
        requestId: bookmarkDeleteBatchRequestId.value,
        deletedIds,
        failedIds,
      };
      options.notify(
        failedIds.length
          ? `已删除 ${deletedIds.length} 个书签，${failedIds.length} 个失败。`
          : `已删除 ${deletedIds.length} 个书签。`,
        failedIds.length ? "error" : "success",
      );
    } finally {
      bookmarkDeleteBatchBusy.value = false;
    }
  }

  async function clearReadingHistoryData(): Promise<void> {
    if (options.isBackupRestoreInProgress()) return;
    const restoreRevision = options.getRestoreRevision();
    const historyRevision = ++historySnapshotRevision;
    try {
      const descriptor = await clearReadingHistory();
      const resource = await readResource<ReadingHistoryResource>(descriptor);
      if (restoreRevision !== options.getRestoreRevision() || historyRevision !== historySnapshotRevision
        || options.isBackupRestoreInProgress()) return;
      readingHistory.value = resource;
      await refreshCurrentReadingStatistics();
      options.notify("阅读记录已清空。");
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`清空阅读记录失败：${options.errorText(error)}`, "error");
      }
    }
  }

  async function removeBookHistory(bookId: string): Promise<void> {
    if (options.isBackupRestoreInProgress()) return;
    const restoreRevision = options.getRestoreRevision();
    const historyRevision = ++historySnapshotRevision;
    try {
      const descriptor = await deleteReadingHistoryForBook(bookId);
      const resource = await readResource<ReadingHistoryResource>(descriptor);
      if (restoreRevision !== options.getRestoreRevision() || historyRevision !== historySnapshotRevision
        || options.isBackupRestoreInProgress()) return;
      readingHistory.value = resource;
      await refreshCurrentReadingStatistics();
      options.notify("这本书的阅读记录已删除。");
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`删除阅读记录失败：${options.errorText(error)}`, "error");
      }
    }
  }

  return {
    bookmarks,
    readingHistory,
    readingStatisticsPeriod,
    readingStatisticsData,
    readingStatisticsBusy,
    readingStatisticsError,
    bookmarkEditorOpen,
    bookmarkNote,
    bookmarkEditingId,
    bookmarkBusy,
    bookmarkDeleteBatchBusy,
    bookmarkDeleteBatchResult,
    currentBookmark,
    refreshBookmarksAndReadingHistory,
    applyBookmarksResource,
    applyReadingHistoryResource,
    refreshCurrentReadingStatistics,
    changeReadingStatisticsPeriod,
    continueReading,
    toggleCurrentBookmark,
    editCurrentBookmarkNote,
    closeBookmarkEditor,
    beginReadingSession,
    finishReadingSession,
    openBookmark,
    saveBookmark,
    removeBookmark,
    removeBookmarks,
    clearReadingHistoryData,
    removeBookHistory,
  };
}
