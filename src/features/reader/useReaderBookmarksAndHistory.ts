import type { ReaderPosition } from "./readerPosition";
import { computed, ref, type Ref } from "vue";
import type { Bookmark, BookResource } from "../../api/types";
import { history, removeHistoryBookmark, saveHistoryBookmark, recordReadingSession } from "../settings/historyStore";
import { openedBook, openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";
export function useReaderBookmarksAndHistory(options: {
  readingChapterIndex: Ref<number>;
  getReaderCurrentPosition: () => ReaderPosition;
  getReaderCurrentOffset: () => number; isBackupRestoreInProgress: () => boolean;
  getRestoreRevision: () => number;
  startReading: (book: BookResource, index?: number, offset?: number, positionHint?: ReaderPosition) => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void; errorText: (error: unknown) => string;
}) {
  const bookmarkEditorOpen = ref(false), bookmarkNote = ref(""), bookmarkEditingId = ref<string | null>(null);
  const currentBookmark = computed(() => (history.value.bookmarks[openedBook.value?.id ?? ""] ?? [])
    .find(record => record.chapterIndex === options.readingChapterIndex.value && record.offset === options.getReaderCurrentOffset() && record.pageId === options.getReaderCurrentPosition().pageId && record.anchor === options.getReaderCurrentPosition().anchor) ?? null);
  let readingSession: { bookId: string; sessionId: string; startedAtMs: number; restoreRevision: number } | null = null;
  function closeBookmarkEditor(): void { bookmarkEditorOpen.value = false; bookmarkEditingId.value = null; bookmarkNote.value = ""; }
  function toggleCurrentBookmark(): void {
    const bookId = openedBook.value?.id;
    if (!bookId || options.isBackupRestoreInProgress()) return;
    if (currentBookmark.value) void removeHistoryBookmark(bookId, currentBookmark.value.id).catch(error=>options.notify(options.errorText(error),"error"));
    else { bookmarkEditingId.value = null; bookmarkNote.value = ""; bookmarkEditorOpen.value = true; }
  }
  function editCurrentBookmarkNote(): void {
    if (!currentBookmark.value) return;
    bookmarkEditingId.value = currentBookmark.value.id; bookmarkNote.value = currentBookmark.value.note; bookmarkEditorOpen.value = true;
  }
  function beginReadingSession(): void {
    const bookId = openedBook.value?.id;
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

  function saveBookmark(): void {
    const book = openedBook.value;
    const chapter = book?.chapters.find(entry => entry.index === options.readingChapterIndex.value);
    if (!book || !chapter || options.isBackupRestoreInProgress()) return;
    const now=Date.now();
    const previous=bookmarkEditingId.value?(history.value.bookmarks[book.id] ?? []).find(record=>record.id===bookmarkEditingId.value):undefined;
    if(bookmarkEditingId.value && !previous){closeBookmarkEditor();return;}
    const bookmark=previous?{...previous,note:bookmarkNote.value.trim(),updatedAtMs:now}:
      {id:crypto.randomUUID(),chapterId:chapter.id,chapterIndex:chapter.index,chapterTitle:chapter.title,...options.getReaderCurrentPosition(),note:bookmarkNote.value.trim(),createdAtMs:now,updatedAtMs:now};
    void saveHistoryBookmark(book.id,bookmark).catch(error=>options.notify(options.errorText(error),"error"));
    closeBookmarkEditor();
  }
  return { bookmarkEditorOpen, bookmarkNote, bookmarkEditingId, currentBookmark, toggleCurrentBookmark,
    editCurrentBookmarkNote, closeBookmarkEditor, saveBookmark, beginReadingSession, finishReadingSession, openBookmark };
}
