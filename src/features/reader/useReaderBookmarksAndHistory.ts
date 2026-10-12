import { computed, ref, type Ref } from "vue";
import type { Bookmark, BookResource } from "../../api/types";
import { history, removeHistoryBookmark } from "../settings/historyStore";
import { openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";
export function useReaderBookmarksAndHistory(options: {
  readingBook: Ref<BookResource | null>; readingChapterIndex: Ref<number>;
  getReaderCurrentOffset: () => number; isBackupRestoreInProgress: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, index?: number, offset?: number) => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void; errorText: (error: unknown) => string;
}) {
  const bookmarkEditorOpen = ref(false), bookmarkNote = ref(""), bookmarkEditingId = ref<string | null>(null);
  const currentBookmark = computed(() => (history.value.bookmarks[options.readingBook.value?.id ?? ""] ?? [])
    .find(record => record.chapterIndex === options.readingChapterIndex.value && record.offset === options.getReaderCurrentOffset()) ?? null);
  const readingSessionId = ref<string | null>(null), readingSessionStartedAt = ref<number | null>(null);
  function closeBookmarkEditor(): void { bookmarkEditorOpen.value = false; bookmarkEditingId.value = null; bookmarkNote.value = ""; }
  function toggleCurrentBookmark(): void {
    const bookId = options.readingBook.value?.id;
    if (!bookId || options.isBackupRestoreInProgress()) return;
    if (currentBookmark.value) removeHistoryBookmark(bookId, currentBookmark.value.id);
    else { bookmarkEditingId.value = null; bookmarkNote.value = ""; bookmarkEditorOpen.value = true; }
  }
  function editCurrentBookmarkNote(): void {
    if (!currentBookmark.value) return;
    bookmarkEditingId.value = currentBookmark.value.id; bookmarkNote.value = currentBookmark.value.note; bookmarkEditorOpen.value = true;
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
    history.value.reading.sessions.push({
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


  function saveBookmark(): void {
    const book = options.readingBook.value;
    const chapter = book?.chapters.find(entry => entry.index === options.readingChapterIndex.value);
    if (!book || !chapter || options.isBackupRestoreInProgress()) return;
    const records = history.value.bookmarks[book.id] ??= [];
    const now = Date.now();
    if (bookmarkEditingId.value) {
      const record = records.find(entry => entry.id === bookmarkEditingId.value);
      if (!record) { closeBookmarkEditor(); return; }
      record.note = bookmarkNote.value.trim(); record.updatedAtMs = now;
    } else {
      records.push({ id: globalThis.crypto.randomUUID(), chapterId: chapter.id, chapterIndex: chapter.index,
        chapterTitle: chapter.title, offset: options.getReaderCurrentOffset(), note: bookmarkNote.value.trim(), createdAtMs: now, updatedAtMs: now });
    }
    closeBookmarkEditor();
  }
  return { bookmarkEditorOpen, bookmarkNote, bookmarkEditingId, currentBookmark, toggleCurrentBookmark,
    editCurrentBookmarkNote, closeBookmarkEditor, saveBookmark, beginReadingSession, finishReadingSession, openBookmark };
}
