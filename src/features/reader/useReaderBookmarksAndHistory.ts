import type { ReaderPosition } from "./readerPosition";
import { computed, type Ref } from "vue";
import type { Bookmark, BookResource } from "../../api/types";
import { history, removeHistoryBookmark, saveHistoryBookmark, recordReadingSession } from "../settings/historyStore";
import { openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";
import { readingBook } from "./readerBookState";
export function useReaderBookmarksAndHistory(options: {
  readingChapterIndex: Ref<number>;
  getReaderCurrentPosition: () => ReaderPosition;
  getReaderCurrentOffset: () => number; isBackupRestoreInProgress: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, index?: number, offset?: number, positionHint?: ReaderPosition) => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void; errorText: (error: unknown) => string;
}) {
  const currentBookmark = computed(() => (history.value.bookmarks[readingBook.value?.id ?? ""] ?? [])
    .find(record => record.chapterIndex === options.readingChapterIndex.value && record.offset === options.getReaderCurrentOffset() && record.pageId === options.getReaderCurrentPosition().pageId && record.anchor === options.getReaderCurrentPosition().anchor) ?? null);
  let readingSession: { bookId: string; sessionId: string; startedAtMs: number; restoreRevision: number } | null = null;
  let bookmarkBusy = false;
  async function toggleCurrentBookmark(): Promise<void> {
    const book = readingBook.value;
    const chapter = book?.chapters.find(entry => entry.index === options.readingChapterIndex.value);
    if (!book || !chapter || bookmarkBusy || options.isBackupRestoreInProgress()) return;
    const restoreRevision = options.getRestoreRevision();
    const previous = currentBookmark.value;
    bookmarkBusy = true;
    try {
      if (previous) await removeHistoryBookmark(book.id, previous.id);
      else {
        const now = Date.now();
        await saveHistoryBookmark(book.id, {
          id: crypto.randomUUID(), chapterId: chapter.id, chapterIndex: chapter.index, chapterTitle: chapter.title,
          ...options.getReaderCurrentPosition(), note: "", createdAtMs: now, updatedAtMs: now,
        });
      }
      if (readingBook.value?.id === book.id && restoreRevision === options.getRestoreRevision()) {
        options.notify(previous ? "已移除书签" : "已添加书签", "success");
      }
    } catch (error) { options.notify(options.errorText(error), "error"); }
    finally { bookmarkBusy = false; }
  }
  function beginReadingSession(): void {
    const bookId = readingBook.value?.id;
    const restoreRevision = options.getRestoreRevision();
    if (!bookId || readingSession && readingSession.bookId === bookId && readingSession.restoreRevision === restoreRevision) return;
    readingSession = { bookId, sessionId: crypto.randomUUID(), startedAtMs: Date.now(), restoreRevision };
  }

  async function finishReadingSession(): Promise<void> {
    if (options.isBackupRestoreInProgress()) return;
    const session = readingSession;
    readingSession = null;
    if (!session || session.restoreRevision !== options.getRestoreRevision()) return;
    const endedAtMs = Date.now();
    const durationMs = endedAtMs - session.startedAtMs;
    if (durationMs <= 0) return;
    await recordReadingSession({
      bookId: session.bookId,
      sessionId: session.sessionId,
      durationMs,
      endedAtMs,
      utcOffsetMinutes: -new Date(endedAtMs).getTimezoneOffset(),
    });
  }

  async function openBookmark(bookmark: Bookmark): Promise<void> {
    if (bookmark.orphaned) {
      options.notify(`“${bookmark.chapterTitle || `第 ${bookmark.chapterIndex + 1} 章`}”无法与新源目录匹配，书签仍保留在列表中。`, "error");
      return;
    }
    try {
      openedBookFromShelf.value = false;
      const book = await switchOpenedBook(bookmark.bookId);
      if (book) await options.startReading(book, bookmark.chapterIndex, bookmark.offset, bookmark);
    } catch (error) {
      options.notify(`打开书签失败：${options.errorText(error)}`, "error");
    }
  }

  return { currentBookmark, toggleCurrentBookmark, beginReadingSession, finishReadingSession, openBookmark };
}
