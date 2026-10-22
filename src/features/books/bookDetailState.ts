import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { saveBook } from "../../api/books";
import { moreInformation } from "../../api/moreInformation";
import type { LoginUpdate } from "../../api/sources";
import { applyLibraryProgress, clearLibraryProgress, libraryType, refreshContentLibrary } from "./contentLibraryState";
import { readBook as queryBook, readBookPreview, readChapter } from "../../api/database";
import type { AppBootstrap, BookPreview, BookResource, BookIdentity, ChapterResource } from "../../api/types";

export const openedBook = ref<BookResource | null>(null);
export const openedBookFromShelf = ref(false);
export const bookPreviews = ref<Record<string, BookPreview>>({});
type BookReference = string | BookIdentity;

let requestId = 0;
let stopWatch: WatchStopHandle = () => {};
const previewLoads = new Map<string, Promise<void>>();
const updates = new Map<string, symbol>();
const latestProgress = new Map<string, NonNullable<BookResource["progress"]>>();
const chapterLoads = new Map<string, { request: number; dirty: boolean; task: Promise<ChapterResource | null> }>();
const typeLoads = new Map<string, { request: number; task: Promise<BookResource | null> }>();
const chapterPatches = new Map<string, Map<string, { revision: number; chapter: ChapterResource }>>();
let chapterRevision = 0;

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
      await updateBook(bookId);
      if (libraryType(bookId)) await refreshContentLibrary();
    }).catch(console.error);
  }, { deep: true });
}

async function updateBook(reference: BookReference): Promise<BookResource | null> {
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
    const revision = chapterRevision;
    const book = await readBook(reference);
    await nextTick();
    if (current !== bookPreviews.value || updates.get(id) !== token) return null;
    if (openedBook.value?.id === id) {
      if (currentRequest !== requestId) return null;
      // 整书快照读取期间到达的章节更新不能被旧快照覆盖。
      for (const patch of chapterPatches.get(id)?.values() ?? []) {
        if (patch.revision <= revision) continue;
        const index = book.chapters.findIndex(chapter => chapter.id === patch.chapter.id);
        if (index >= 0 && book.chapters[index]!.index === patch.chapter.index) book.chapters[index] = patch.chapter;
      }
      chapterPatches.delete(id);
      stopWatch(); openedBook.value = book; watchBook();
    }
    updatePreview(book);
    return openedBook.value?.id === id ? openedBook.value : null;
  } finally { if (updates.get(id) === token) updates.delete(id); }
}

/** 缓存通知只替换对应章节，不重读目录、元数据或整个书架。 */
export function refreshOpenedChapter(bookId: string, chapterId: string): Promise<ChapterResource | null> {
  const book = openedBook.value, currentRequest = requestId;
  if (!book || book.id !== bookId) return Promise.resolve(null);
  const key = `${bookId}:${chapterId}`, pending = chapterLoads.get(key);
  if (pending?.request === currentRequest) { pending.dirty = true; return pending.task; }
  const load = { request: currentRequest, dirty: false, task: null as unknown as Promise<ChapterResource | null> };
  const task = (async () => {
    let chapter: ChapterResource | null = null;
    do {
      load.dirty = false;
      const snapshot = await readChapter(book, chapterId);
      const current = openedBook.value;
      if (!snapshot || currentRequest !== requestId || current?.id !== bookId) return null;
      // 正文规则把文本书识别为 HTML 时，其他章节引用也会失效，需要同步整书类型。
      if (snapshot.contentType !== current.contentType) {
        let refresh = typeLoads.get(bookId);
        if (refresh?.request !== currentRequest) {
          const task = updateBook(bookId).finally(() => { if (typeLoads.get(bookId)?.task === task) typeLoads.delete(bookId); });
          refresh = { request: currentRequest, task };
          typeLoads.set(bookId, refresh);
        }
        const updated = await refresh.task;
        if (load.dirty) continue;
        return updated?.chapters.find(entry => entry.id === chapterId) ?? null;
      }
      chapter = snapshot.chapter;
      const index = current.chapters.findIndex(entry => entry.id === chapterId);
      if (index < 0 || current.chapters[index]!.index !== chapter.index) return null;
      current.chapters[index] = chapter;
      if (updates.has(bookId)) {
        let patches = chapterPatches.get(bookId);
        if (!patches) chapterPatches.set(bookId, patches = new Map());
        patches.set(chapterId, { revision: ++chapterRevision, chapter });
      }
    } while (load.dirty);
    return chapter;
  })().finally(() => { if (chapterLoads.get(key)?.task === task) chapterLoads.delete(key); });
  load.task = task;
  chapterLoads.set(key, load);
  return task;
}

export async function initializeBookStore(): Promise<void> {
  // 登录规则触发详情刷新时，仍走统一详情入口，再从数据库读取。
  await listen<LoginUpdate>("login", ({ payload }) => {
    const book = openedBook.value;
    if (!payload.refreshTargets?.includes("book") || !book?.bookUrl || book.sourceId !== payload.sourceId) return;
    void moreInformation(payload.sourceId, book.bookUrl).then(() => updateBook(book.id)).catch(console.error);
  });
  await listen<{ bookId: string; progress: NonNullable<BookResource["progress"]>; read: number }>("reading-progress-updated", ({ payload }) => {
    const { bookId, progress, read } = payload;
    if ((latestProgress.get(bookId)?.updatedAtMs ?? 0) > (progress.updatedAtMs ?? 0)) return;
    latestProgress.set(bookId, progress);
    if (openedBook.value?.id === bookId && (openedBook.value.progress?.updatedAtMs ?? 0) <= (progress.updatedAtMs ?? 0)) openedBook.value.progress = progress;
    const preview = bookPreviews.value[bookId];
    if (preview && (preview.progress?.updatedAtMs ?? 0) <= (progress.updatedAtMs ?? 0)) preview.progress = progress;
    applyLibraryProgress(bookId, progress.updatedAtMs ?? 0, read);
  });
  await listen<BookIdentity & { chapterId?: string }>("book-updated", ({ payload }) => {
    const id = referenceId(payload);
    if (payload.chapterId) {
      void refreshOpenedChapter(id, payload.chapterId).catch(console.error);
      return;
    }
    latestProgress.delete(id);
    clearLibraryProgress(id);
    if (libraryType(id)) void refreshContentLibrary().catch(console.error);
    if (openedBook.value?.id === id || bookPreviews.value[id] || previewLoads.has(id)) {
      void (async () => {
        await previewLoads.get(id);
        await updateBook(payload);
      })().catch(console.error);
    }
  });
  await listen<AppBootstrap>("app-state-updated", () => {
    clearOpenedBook(); bookPreviews.value = {};
    previewLoads.clear(); updates.clear(); latestProgress.clear(); chapterLoads.clear(); typeLoads.clear(); chapterPatches.clear();
  });
}

export const refreshOpenedBook = (reference: BookReference) => updateBook(reference);

export function clearBookProgress(id: string): void {
  latestProgress.delete(id);
  clearLibraryProgress(id);
}

export function replaceOpenedBook(book: BookResource | null): void {
  chapterPatches.clear();
  requestId++; stopWatch(); openedBook.value = book; watchBook();
  if (book) updatePreview(book);
}

export function closeBookDetail(preserveBook = false): void {
  if (!preserveBook) replaceOpenedBook(null);
}
export const clearOpenedBook = () => closeBookDetail();

export async function switchOpenedBook(reference: BookReference): Promise<BookResource | null> {
  const current = bookPreviews.value, id = ++requestId;
  stopWatch(); openedBook.value = null; chapterPatches.clear();
  const book = await readBook(reference);
  if (current !== bookPreviews.value || id !== requestId) return null;
  openedBook.value = book; watchBook(); updatePreview(book);
  return openedBook.value;
}
