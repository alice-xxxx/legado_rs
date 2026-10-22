import type { ReaderPosition } from "./readerPosition";
import type { ReaderMediaElement } from "./nativeAudio";
import { openedBook } from "../books/bookDetailState";
import { nextTick, ref, watch, type Ref } from "vue";
import { readTextResource } from "../../api/resources";
import { bookContentType } from "../../api/bookContentType";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import type { BookResource, ChapterImagesResource, ChapterMediaResource, ChapterPdfPageResource, ChapterRichTextResource, ChapterTextResource } from "../../api/types";
import type { PdfPageResource } from "./readerContent";
import type { ReaderPendingFragmentState } from "./useReaderPageInteraction";
import type { ReaderTextChapter } from "./readerContent";
export type CachedChapterContent = ChapterTextResource | ChapterRichTextResource | ChapterImagesResource | ChapterMediaResource | ChapterPdfPageResource;

export interface ReaderChapterLoadingOptions {
  reader: {
    chapterIndex: Ref<number>;
    pendingPosition: Ref<ReaderPosition | null>;
    content: Ref<CachedChapterContent | null>;
    pdfPage: Readonly<Ref<PdfPageResource | null>>;
    pdfZoomOverride: Ref<PdfPageResource["defaultZoom"] | null>;
    imagePage: Readonly<Ref<boolean>>;
    mediaChapter: Readonly<Ref<boolean>>;
    mediaTimeMs: Ref<number>;
    mediaDurationMs: Ref<number>;
    mediaError: Ref<string>;
    mediaRetrying: Ref<boolean>;
    pageIndex: Ref<number>;
    pageCount: Ref<number>;
    comicScrollTop: Ref<number>;
    visible: Ref<boolean>;
    controlsOpen: Ref<boolean>;
    directoryOpen: Ref<boolean>;
    controlsMode: Ref<"settings" | "tts" | "appearance" | "search">;
    menuOpen: Ref<boolean>;
    busy: Ref<boolean>;
    progressDirty: Ref<boolean>;
    getMediaElement: () => ReaderMediaElement | null;
    getExpectedTtsChapter: () => string | null | undefined;
    getLoadRevision: () => number;
    advanceLoadRevision: () => number;
    advanceProgressRevision: () => void;
    setPendingFragment: (value: ReaderPendingFragmentState | null) => void;
    resetClickSuppression: () => void;
    chapterError: Ref<string>;
  };
  actions: {
    notifyCurrentReading: (refresh?: boolean, prepareCurrent?: boolean) => Promise<void>;
    showShelfBehindReader: (book: BookResource) => Promise<void>;
    stopTts: (showNotice?: boolean) => void;
    pauseReaderMediaAndSave: () => Promise<boolean>;
    clearReaderMediaBinding: (pause?: boolean) => void;
    clearReaderFindMatches: () => void;
    clearReaderScrollListeners: () => void;
    applyPendingReaderFragment: () => void;
    applyComicScrollPosition: () => void;
    restoreReadingPosition: (position: ReaderPosition) => void;
    captureReadingPosition: () => ReaderPosition;
    measureReaderPages: () => void;
    beginReadingSession: () => void;
    isBackupRestoreInProgress: () => boolean;
    errorText: (error: unknown) => string;
    notify: (message: string, kind?: "success" | "error") => void;
  };
}

/** 管理阅读器章节准备、读取与媒体重试。 */
export function useReaderChapterLoading({ reader, actions }: ReaderChapterLoadingOptions) {
  const request = ref(0);
  let finishLoad: ((loaded: boolean) => void) | null = null;
  let adoptedIndex: number | null = null;
  function complete(loaded: boolean): void {
    reader.busy.value = false;
    const finish = finishLoad;
    finishLoad = null;
    finish?.(loaded);
  }
  function failPending(error: string): void {
    reader.chapterError.value = error;
    reader.menuOpen.value = true;
    complete(false);
  }
  watch(() => {
    if (!reader.visible.value) return null;
    const book = openedBook.value;
    const chapter = book?.chapters.find(entry => entry.index === reader.chapterIndex.value);
    if (!book || !chapter) return null;
    const type = bookContentType(book);
    const ready = type === "image" ? chapter.pages?.length && JSON.stringify(chapter.pages.map(page => [page.id, page.src])) : type === "audio" || type === "video"
      ? chapter.media?.src : type === "pdf" ? chapter.pdfPage?.src : chapter.resource?.src;
    if (!ready) return null;
    return JSON.stringify([book.id, type, chapter.id, ready, request.value]);
  }, async (key, _previous, onCleanup) => {
    if (!key) { if (!reader.visible.value) complete(false); return; }
    if (adoptedIndex === reader.chapterIndex.value && reader.content.value) {
      adoptedIndex = null;
      return;
    }
    const book = openedBook.value!;
    const chapter = book.chapters.find(entry => entry.index === reader.chapterIndex.value)!;
    const controller = new AbortController();
    onCleanup(() => controller.abort());
    if (!reader.pendingPosition.value) reader.pendingPosition.value = actions.captureReadingPosition();
    reader.busy.value = true;
    actions.clearReaderFindMatches();
    actions.clearReaderScrollListeners();
    actions.clearReaderMediaBinding(true);
    try {
      const type = bookContentType(book);
      let content: CachedChapterContent;
      if (type === "image") content = { kind: "images", pages: chapter.pages! };
      else if (type === "audio" || type === "video") content = chapter.media!;
      else if (type === "pdf") content = chapter.pdfPage!;
      else {
        const body = await readTextResource(chapter.resource!, controller.signal);
        content = type === "html" ? { schemaVersion: 1, kind: "richText", markup: body } : { schemaVersion: 1, kind: "text", text: body };
      }
      if (controller.signal.aborted || actions.isBackupRestoreInProgress()) return;
      reader.content.value = content;
      reader.chapterError.value = "";
      reader.busy.value = false;
      await nextTick();
      if (controller.signal.aborted) return;
      if (!reader.imagePage.value && !reader.mediaChapter.value && !reader.pdfPage.value) actions.measureReaderPages();
      if (!reader.mediaChapter.value && !reader.pdfPage.value && reader.pendingPosition.value) {
        actions.restoreReadingPosition(reader.pendingPosition.value);
      }
      reader.pendingPosition.value = null;
      actions.beginReadingSession();
      actions.applyPendingReaderFragment();
      complete(true);
    } catch (error) {
      if (!controller.signal.aborted) {
        failPending(actions.errorText(error));
      }
    }
  });

  async function loadReaderChapter(index: number, offset?: number, fragment?: string, positionHint?: ReaderPosition): Promise<boolean> {
    adoptedIndex = null;
    const book = openedBook.value;
    const chapter = book?.chapters.find(entry => entry.index === index);
    if (!book || !chapter || chapter.volume || actions.isBackupRestoreInProgress()) return false;
    if (reader.getMediaElement() && !(await actions.pauseReaderMediaAndSave())) return false;
    complete(false);
    if (reader.getExpectedTtsChapter() !== book.id + ":" + index) actions.stopTts(false);
    actions.clearReaderMediaBinding(true);
    actions.clearReaderFindMatches();
    actions.clearReaderScrollListeners();
    const saved = book.progress?.chapterIndex === index ? book.progress : undefined;
    const position = Math.max(0, offset ?? saved?.offset ?? 0);
    reader.pendingPosition.value = positionHint ?? { offset: position, pageId: saved?.pageId, anchor: saved?.anchor, quality: saved?.quality };
    reader.advanceLoadRevision();
    reader.chapterIndex.value = index;
    reader.pageIndex.value = 0;
    reader.pageCount.value = 1;
    reader.comicScrollTop.value = 0;
    reader.mediaTimeMs.value = position;
    reader.mediaDurationMs.value = 0;
    reader.mediaError.value = "";
    reader.content.value = null;
    reader.chapterError.value = "";
    reader.busy.value = true;
    reader.progressDirty.value = false;
    reader.advanceProgressRevision();
    reader.setPendingFragment(fragment != null ? { fragment, loadRevision: reader.getLoadRevision(), chapterIndex: index } : null);
    const done = new Promise<boolean>(resolve => { finishLoad = resolve; });
    const revision = reader.getLoadRevision();
    void actions.notifyCurrentReading(false, true)
      .catch(error => { if (reader.getLoadRevision() === revision && openedBook.value?.id === book.id) failPending(actions.errorText(error)); });
    request.value += 1;
    return done;
  }
  async function startReading(book: BookResource, index?: number, offset?: number, positionHint?: ReaderPosition): Promise<void> {
    await actions.showShelfBehindReader(book);
    reader.visible.value = true;
    reader.controlsOpen.value = false;
    reader.directoryOpen.value = false;
    reader.controlsMode.value = "settings";
    reader.menuOpen.value = false;
    reader.pdfZoomOverride.value = null;
    reader.resetClickSuppression();
    const requested = index ?? book.progress?.chapterIndex ?? 0;
    const target = book.chapters.find(chapter => chapter.index >= requested && !chapter.volume)
      ?? book.chapters.find(chapter => !chapter.volume);
    if (!target) { reader.chapterError.value = "没有可阅读的内容。"; reader.menuOpen.value = true; return; }
    await loadReaderChapter(target.index, target.index === requested
      ? offset ?? (index == null ? book.progress?.offset ?? 0 : 0) : 0, undefined,
      target.index === requested ? positionHint : undefined);
  }
  async function reloadCurrentChapter(): Promise<void> {
    const book = openedBook.value;
    const chapter = book?.chapters.find(entry => entry.index === reader.chapterIndex.value);
    if (!book || !chapter || !reader.visible.value || reader.busy.value || reader.mediaRetrying.value
      || actions.isBackupRestoreInProgress() || shelfBatchRecoveryRequired.value) return;
    if (!(await actions.pauseReaderMediaAndSave())) return;
    if (openedBook.value?.id !== book.id || reader.chapterIndex.value !== chapter.index) return;
    reader.mediaRetrying.value = true;
    try {
      await actions.notifyCurrentReading(true);
      if (reader.visible.value && openedBook.value?.id === book.id && reader.chapterIndex.value === chapter.index) {
        request.value += 1;
      }
    } catch (error) { reader.mediaError.value = actions.errorText(error); actions.notify(reader.mediaError.value, "error"); }
    finally { reader.mediaRetrying.value = false; }
  }
  function adoptPreparedChapter(chapter: ReaderTextChapter): void {
    adoptedIndex = chapter.index;
    reader.chapterIndex.value = chapter.index;
    reader.content.value = chapter.content;
    reader.advanceLoadRevision();
    reader.progressDirty.value = true;
    reader.advanceProgressRevision();
  }
  return { startReading, loadReaderChapter, reloadCurrentChapter, adoptPreparedChapter, failPending, cancel: () => complete(false) };
}
