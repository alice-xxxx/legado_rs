import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { saveBook } from "../../api/books";
import { applyLibraryProgress, clearLibraryProgress, libraryType, refreshContentLibrary } from "./contentLibraryState";
import { readBook as queryBook, readBookPreview } from "../../api/database";
import type { AppBootstrap, BookPreview, BookResource, BookIdentity } from "../../api/types";

export const openedBook = ref<BookResource | null>(null);
export const openedBookFromShelf = ref(false);
export const bookPreviews = ref<Record<string, BookPreview>>({});
type BookReference = string | BookIdentity;
type BookUpdatedEvent = { requestId: number; resource: BookIdentity };
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};
const previewLoads = new Map<string, Promise<void>>();
const updates = new Map<string, symbol>();
const latestProgress = new Map<string, NonNullable<BookResource["progress"]>>();

function referenceId(reference: BookReference): string {
  if (typeof reference === "string") return reference;
  const id = reference.bookId;
  if (!id) throw new Error("书籍 ID 无效。");
  return id;
}

// Opened books and previews share one state layer over the read-only database.
async function readBook(reference: BookReference): Promise<BookResource> {
  const id = referenceId(reference);
  const book = await queryBook(id);
  if (book.id !== id) throw new Error("读取的书籍与缓存 ID 不匹配。");
  const progress = latestProgress.get(id);
  if (progress && (progress.updatedAtMs ?? 0) > (book.progress?.updatedAtMs ?? 0)) book.progress = progress;
  return book;
}

function previewOf(book: BookResource): BookPreview {
  return {
    id: book.id, title: book.title, author: book.author, coverSrc: book.coverSrc,
    kind: book.kind, contentType: book.contentType, mediaType: book.mediaType, chapterCount: book.chapterCount,
    latestChapter: book.latestChapter, canChangeSource: book.canChangeSource,
    sourceId: book.sourceId, sourceName: book.sourceName,
    progress: book.progress ? {
      chapterId: book.progress.chapterId, chapterIndex: book.progress.chapterIndex,
      offset: book.progress.offset, updatedAtMs: book.progress.updatedAtMs,
    } : undefined,
  };
}

function updatePreview(book: BookResource): void {
  if (bookPreviews.value[book.id]) bookPreviews.value[book.id] = previewOf(book);
}

export async function loadBookPreview(reference: BookReference): Promise<BookPreview | undefined> {
  const id = referenceId(reference);
  if (!bookPreviews.value[id]) {
    if (openedBook.value?.id === id) bookPreviews.value[id] = previewOf(openedBook.value);
    else {
      let pending = previewLoads.get(id);
      if (!pending) {
        const current = bookPreviews.value;
        pending = readBookPreview(id).then(book => {
          const progress = latestProgress.get(id);
          if (progress && (progress.updatedAtMs ?? 0) > (book.progress?.updatedAtMs ?? 0)) book.progress = progress;
          if (current === bookPreviews.value && !current[id]) current[id] = book;
        }).finally(() => { if (previewLoads.get(id) === pending) previewLoads.delete(id); });
        previewLoads.set(id, pending);
      }
      await pending;
    }
  }
  return bookPreviews.value[id];
}

export async function loadBookPreviews(ids: string[]): Promise<void> {
  // Keep complete catalogs transient instead of loading every book concurrently.
  const current = bookPreviews.value;
  for (const id of new Set(ids)) {
    if (current !== bookPreviews.value) return;
    try { await loadBookPreview(id); } catch (error) { console.error(error); }
  }
}

function watchBook(): void {
  stopWatch = watch(() => openedBook.value?.displayOverrides, displayOverrides => {
    if (!openedBook.value) return;
    const id = ++requestId;
    const bookId = openedBook.value.id;
    void saveBook({
      id: openedBook.value.id,
      displayOverrides: {
        title: displayOverrides?.title, author: displayOverrides?.author,
        intro: displayOverrides?.intro, coverSrc: displayOverrides?.coverSrc,
      },
    }).then(async () => {
      if (id !== requestId) return;
      await updateBook(bookId, id);
      if (libraryType(bookId)) await refreshContentLibrary();
    }).catch(console.error);
  }, { deep: true });
}

async function updateBook(reference: BookReference, eventRequestId = 0): Promise<BookResource | null> {
  if (eventRequestId !== 0 && eventRequestId !== requestId) return null;
  const id = referenceId(reference), token = Symbol();
  const current = bookPreviews.value, currentRequest = requestId;
  updates.set(id, token);
  try {
    if (openedBook.value?.id !== id) {
      const preview = await readBookPreview(id);
      const progress = latestProgress.get(id);
      if (progress && (progress.updatedAtMs ?? 0) > (preview.progress?.updatedAtMs ?? 0)) preview.progress = progress;
      if (current === bookPreviews.value && updates.get(id) === token && current[id]) current[id] = preview;
      return null;
    }
    const book = await readBook(reference);
    await nextTick();
    if (current !== bookPreviews.value || updates.get(id) !== token) return null;
    if (openedBook.value?.id === id) {
      if (currentRequest !== requestId) return null;
      stopWatch(); openedBook.value = book; watchBook();
    }
    updatePreview(book);
    return openedBook.value?.id === id ? openedBook.value : null;
  } finally { if (updates.get(id) === token) updates.delete(id); }
}

export async function initializeBookStore(): Promise<void> {
  await listen<{ bookId: string; progress: NonNullable<BookResource["progress"]>; read: number }>("reading-progress-updated", ({ payload }) => {
    const { bookId, progress, read } = payload;
    if ((latestProgress.get(bookId)?.updatedAtMs ?? 0) > (progress.updatedAtMs ?? 0)) return;
    latestProgress.set(bookId, progress);
    if (openedBook.value?.id === bookId && (openedBook.value.progress?.updatedAtMs ?? 0) <= (progress.updatedAtMs ?? 0)) openedBook.value.progress = progress;
    const preview = bookPreviews.value[bookId];
    if (preview && (preview.progress?.updatedAtMs ?? 0) <= (progress.updatedAtMs ?? 0)) preview.progress = progress;
    applyLibraryProgress(bookId, progress.updatedAtMs ?? 0, read);
  });
  await listen<BookUpdatedEvent>("book-updated", ({ payload }) => {
    const id = referenceId(payload.resource);
    latestProgress.delete(id);
    clearLibraryProgress(id);
    if (libraryType(id)) void refreshContentLibrary().catch(console.error);
    if (openedBook.value?.id === id || bookPreviews.value[id] || previewLoads.has(id)) {
      void (async () => {
        await previewLoads.get(id);
        await updateBook(payload.resource, payload.requestId);
      })().catch(console.error);
    }
  });
  await listen<AppBootstrap>("app-state-updated", () => {
    clearOpenedBook(); bookPreviews.value = {};
    previewLoads.clear(); updates.clear(); latestProgress.clear();
  });
}

export const refreshOpenedBook = (reference: BookReference) => updateBook(reference);

export function clearBookProgress(id: string): void {
  latestProgress.delete(id);
  clearLibraryProgress(id);
}

export function replaceOpenedBook(book: BookResource | null): void {
  requestId++; stopWatch(); openedBook.value = book; watchBook();
  if (book) updatePreview(book);
}

export function closeBookDetail(preserveBook = false): void {
  if (!preserveBook) replaceOpenedBook(null);
}
export const clearOpenedBook = () => closeBookDetail();

export async function switchOpenedBook(reference: BookReference): Promise<BookResource | null> {
  const current = bookPreviews.value, id = ++requestId;
  stopWatch(); openedBook.value = null;
  const book = await readBook(reference);
  if (current !== bookPreviews.value || id !== requestId) return null;
  openedBook.value = book; watchBook(); updatePreview(book);
  return openedBook.value;
}
