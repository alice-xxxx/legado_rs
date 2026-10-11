import { computed, ref, watch, type Ref } from "vue";
import { readingStatistics } from "../settings/readingStatistics";
import { type Bookmark, type BookResource, type ReadingStatisticsResource } from "../../api/types";
import { history as appHistory, removeHistoryBookmark } from "../settings/historyStore";
import { openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";

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

/** 阅读记录和完整书签都来自全局 history。 */
export function useReaderBookmarksAndHistory(options: ReaderBookmarksAndHistoryOptions) {
  const bookmarks = ref<Bookmark[]>([]);
  const readingHistory = computed(() => appHistory.value.reading);
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

  watch(() => ({
    bookId: options.readingBook.value?.id,
    records: appHistory.value.bookmarks[options.readingBook.value?.id ?? ""],
  }), ({ bookId, records }) => {
    bookmarks.value = bookId ? (records ?? []).map((bookmark) => ({ ...bookmark, bookId })) : [];
  }, { immediate: true, deep: true });

  async function refreshBookBookmarks(bookId = options.readingBook.value?.id): Promise<void> {
    const requestRevision = ++bookmarkSnapshotRevision;
    if (!bookId) { bookmarks.value = []; return; }
    const records = appHistory.value.bookmarks[bookId] ?? [];
    if (requestRevision !== bookmarkSnapshotRevision) return;
    bookmarks.value = records.map((bookmark) => ({ ...bookmark, bookId }));
  }

  async function refreshBookmarksAndReadingHistory(): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return;
    await refreshBookBookmarks();
    if (restoreRevision === options.getRestoreRevision()) await refreshCurrentReadingStatistics();
  }

  async function refreshBookmarksAfterHistoryUpdate(): Promise<void> {
    if (!options.isBackupRestoreInProgress()) await refreshBookBookmarks();
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
      const statistics = readingStatistics(readingHistory.value, fromMs, toMs);
      if (isCurrent()) filteredReadingStatistics.value = statistics;
    } catch (error) {
      if (isCurrent()) readingStatisticsError.value = options.errorText(error);
    } finally {
      if (isCurrent()) readingStatisticsBusy.value = false;
    }
  }

  watch(() => readingHistory.value.sessions, () => { void refreshCurrentReadingStatistics(); }, { deep: true });

  function changeReadingStatisticsPeriod(period: ReadingStatisticsPeriod): void {
    readingStatisticsPeriod.value = period;
    void refreshCurrentReadingStatistics();
  }

  async function continueReading(bookId: string): Promise<void> {
    try {
      openedBookFromShelf.value = false;
      const book = await switchOpenedBook(bookId);
      if (book) await options.startReading(book);
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
    const bookId = options.readingBook.value?.id;
    const sessionId = readingSessionId.value;
    const startedAt = readingSessionStartedAt.value;
    readingSessionId.value = null;
    readingSessionStartedAt.value = null;
    if (!bookId || !sessionId || startedAt == null) return;
    const endedAtMs = Date.now();
    const durationMs = endedAtMs - startedAt;
    if (durationMs <= 0 || restoreRevision !== options.getRestoreRevision()) return;
    appHistory.value.reading.sessions.push({
      bookId,
      sessionId,
      durationMs,
      endedAtMs,
      utcOffsetMinutes: -new Date(endedAtMs).getTimezoneOffset(),
    });
  }

  async function openBookmark(bookmark: Bookmark): Promise<void> {
    if (bookmark.orphaned) {
      options.notify(`“${bookmark.chapterTitle || `第 ${bookmark.chapterIndex + 1} 章`}”无法与新书源目录匹配，书签仍保留在列表中。`, "error");
      return;
    }
    try {
      openedBookFromShelf.value = false;
      const book = await switchOpenedBook(bookmark.bookId);
      if (book) await options.startReading(book, bookmark.chapterIndex, bookmark.offset);
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
      const now = Date.now();
      const records = appHistory.value.bookmarks[book.id] ?? [];
      if (bookmarkToEdit) {
        const record = records.find((entry) => entry.id === bookmarkToEdit.id);
        if (!record) return;
        record.note = note;
        record.updatedAtMs = now;
      } else {
        const chapter = book.chapters[options.readingChapterIndex.value];
        if (!chapter) return;
        records.push({
          id: globalThis.crypto.randomUUID(), chapterId: chapter.id,
          chapterIndex: chapter.index, chapterTitle: chapter.title,
          offset: options.getReaderCurrentOffset(), note,
          createdAtMs: now, updatedAtMs: now,
        });
        appHistory.value.bookmarks[book.id] = records;
      }
      if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return;
      await refreshBookBookmarks(book.id);
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
      const bookmark = bookmarks.value.find((entry) => entry.id === bookmarkId);
      if (!bookmark) return false;
      removeHistoryBookmark(bookmark.bookId, bookmarkId);
      if (restoreRevision !== options.getRestoreRevision() || options.isBackupRestoreInProgress()) return false;
      await refreshBookBookmarks(bookmark.bookId);
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

  function clearReadingHistoryData(): void {
    if (options.isBackupRestoreInProgress()) return;
    appHistory.value.reading = { sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 };
    options.notify("阅读记录已清空。");
  }

  function removeBookHistory(bookId: string): void {
    if (options.isBackupRestoreInProgress()) return;
    appHistory.value.reading.sessions = appHistory.value.reading.sessions
      .filter((session) => session.bookId !== bookId);
    options.notify("这本书的阅读记录已删除。");
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
    refreshBookmarksAfterHistoryUpdate,
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
