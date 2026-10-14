import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { saveBook } from "../../api/books";
import { command } from "../../api/ipc";
import { contentUrl, libraryType, refreshContentLibrary } from "./contentLibraryState";
import { readResource } from "../../api/resources";
import type { AppBootstrap, BookPreview, BookResource, ResourceDescriptor } from "../../api/types";

export const openedBook = ref<BookResource | null>(null);
export const openedBookFromShelf = ref(false);
export const bookPreviews = ref<Record<string, BookPreview>>({});
type BookReference = string | ResourceDescriptor;
type BookUpdatedEvent = { requestId: number; resource: ResourceDescriptor };
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};
const previewLoads = new Map<string, Promise<void>>();
const updates = new Map<string, symbol>();

function referenceId(reference: BookReference): string {
  if (typeof reference === "string") return reference;
  const id = /^(?:resource:\/\/)?cache\/(?:text|html|image|audio|video|pdf)\/([^/]+)\/\1\.json$/.exec(reference.resourceId)?.[1];
  if (!id) throw new Error("书籍资源地址无效。");
  return id;
}

// All book.json reads stay here; callers receive shared state, never a raw document.
async function readBook(reference: BookReference): Promise<BookResource> {
  const id = referenceId(reference);
  const resource = typeof reference === "string"
    ? await command<ResourceDescriptor>("get_book", { bookId: id }) : reference;
  const book = await readResource<BookResource>(resource);
  if (book.id !== id) throw new Error("读取的书籍与缓存 ID 不匹配。");
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
    bookSrc: contentUrl(book.contentType ?? "text", book.id),
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
        pending = readBook(reference).then(book => {
          if (current === bookPreviews.value && !current[id]) current[id] = previewOf(book);
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
    void saveBook({
      id: openedBook.value.id,
      displayOverrides: {
        title: displayOverrides?.title, author: displayOverrides?.author,
        intro: displayOverrides?.intro, coverSrc: displayOverrides?.coverSrc,
      },
    }, ++requestId).catch(console.error);
  }, { deep: true });
}

async function updateBook(reference: BookReference, eventRequestId = 0): Promise<BookResource | null> {
  if (eventRequestId !== 0 && eventRequestId !== requestId) return null;
  const id = referenceId(reference), token = Symbol();
  const current = bookPreviews.value, currentRequest = requestId;
  updates.set(id, token);
  try {
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

export async function initializeBookStore(_descriptor: ResourceDescriptor): Promise<void> {
  await listen<BookUpdatedEvent>("book-updated", ({ payload }) => {
    const id = referenceId(payload.resource);
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
    previewLoads.clear(); updates.clear();
  });
}

export const refreshOpenedBook = (reference: BookReference) => updateBook(reference);

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
