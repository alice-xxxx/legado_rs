import { nextTick, ref, watch, type Ref } from "vue";
import { notifyReading } from "../../api/books";
import { readResource } from "../../api/resources";
import type { BookResource, ChapterMediaResource, ChapterPdfPageResource, ChapterRichTextResource, ChapterTextResource } from "../../api/types";
import type { PdfPageResource } from "./readerContent";
import type { ReaderPendingFragmentState } from "./useReaderPageInteraction";
export type CachedChapterContent = ChapterTextResource | ChapterRichTextResource | ChapterMediaResource | ChapterPdfPageResource;

export interface ReaderChapterLoadingOptions {
  reader: {
    book: Ref<BookResource | null>;
    chapterIndex: Ref<number>;
    content: Ref<CachedChapterContent | null>;
    pdfPage: Readonly<Ref<PdfPageResource | null>>;
    pdfZoomOverride: Ref<PdfPageResource["defaultZoom"] | null>;
    pdfInitialPassword: Ref<string>;
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
    pdfImportPasswords: Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>;
    getMediaElement: () => HTMLMediaElement | null;
    getExpectedTtsChapter: () => string | null | undefined;
    getLoadRevision: () => number;
    advanceLoadRevision: () => number;
    advanceProgressRevision: () => void;
    setPendingFragment: (value: ReaderPendingFragmentState | null) => void;
    resetClickSuppression: () => void;
    chapterError: Ref<string>;
    errorChapterIndex: Ref<number | null>;
  };
  actions: {
    showShelfBehindReader: (book: BookResource) => Promise<void>;
    stopTts: (showNotice?: boolean) => void;
    pauseReaderMediaAndSave: () => Promise<boolean>;
    clearReaderMediaBinding: (pause?: boolean) => void;
    clearReaderFindMatches: () => void;
    clearReaderScrollListeners: () => void;
    applyPendingReaderFragment: () => void;
    applyComicScrollPosition: () => void;
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
  function complete(loaded: boolean): void {
    reader.busy.value = false;
    const finish = finishLoad;
    finishLoad = null;
    finish?.(loaded);
  }
  function failPending(error: string): void {
    reader.chapterError.value = error;
    reader.errorChapterIndex.value = reader.chapterIndex.value;
    reader.menuOpen.value = true;
    complete(false);
  }
  watch(() => {
    if (!reader.visible.value) return null;
    const book = reader.book.value;
    const chapter = book?.chapters.find(entry => entry.index === reader.chapterIndex.value);
    return chapter?.resource ? JSON.stringify([book!.id, chapter.id, chapter.resource.src, request.value]) : null;
  }, async (key, _previous, onCleanup) => {
    if (!key) { if (!reader.visible.value) complete(false); return; }
    const book = reader.book.value!;
    const chapter = book.chapters.find(entry => entry.index === reader.chapterIndex.value)!;
    const controller = new AbortController();
    onCleanup(() => controller.abort());
    reader.busy.value = true;
    actions.clearReaderFindMatches();
    actions.clearReaderScrollListeners();
    actions.clearReaderMediaBinding(true);
    try {
      const content = await readResource<CachedChapterContent>(chapter.resource!, controller.signal);
      if (controller.signal.aborted || actions.isBackupRestoreInProgress()) return;
      reader.content.value = content;
      reader.pdfInitialPassword.value = reader.pdfPage.value ? reader.pdfImportPasswords.get(book.id)?.password ?? "" : "";
      reader.chapterError.value = "";
      reader.errorChapterIndex.value = null;
      reader.busy.value = false;
      await nextTick();
      if (controller.signal.aborted) return;
      if (reader.imagePage.value) actions.applyComicScrollPosition();
      else if (!reader.mediaChapter.value && !reader.pdfPage.value) actions.measureReaderPages();
      actions.beginReadingSession();
      actions.applyPendingReaderFragment();
      complete(true);
    } catch (error) {
      if (!controller.signal.aborted) failPending(actions.errorText(error));
    }
  });

  async function loadReaderChapter(index: number, offset?: number, fragment?: string): Promise<boolean> {
    const book = reader.book.value;
    const chapter = book?.chapters.find(entry => entry.index === index);
    if (!book || !chapter || actions.isBackupRestoreInProgress()) return false;
    if (reader.getMediaElement() && !(await actions.pauseReaderMediaAndSave())) return false;
    complete(false);
    if (reader.getExpectedTtsChapter() !== book.id + ":" + index) actions.stopTts(false);
    actions.clearReaderMediaBinding(true);
    actions.clearReaderFindMatches();
    actions.clearReaderScrollListeners();
    const sameChapter = reader.chapterIndex.value === index;
    const position = Math.max(0, offset ?? (sameChapter ? reader.mediaChapter.value ? reader.mediaTimeMs.value
      : reader.imagePage.value ? reader.comicScrollTop.value : reader.pageIndex.value
      : book.progress?.chapterIndex === index ? book.progress.offset ?? 0 : 0));
    reader.advanceLoadRevision();
    reader.chapterIndex.value = index;
    reader.pageIndex.value = position;
    reader.pageCount.value = 1;
    reader.comicScrollTop.value = position;
    reader.mediaTimeMs.value = position;
    reader.mediaDurationMs.value = 0;
    reader.mediaError.value = "";
    reader.content.value = null;
    reader.chapterError.value = "";
    reader.busy.value = true;
    reader.progressDirty.value = false;
    reader.advanceProgressRevision();
    reader.setPendingFragment(fragment != null ? { fragment, loadRevision: reader.getLoadRevision(), chapterIndex: index, frameLoaded: true } : null);
    const done = new Promise<boolean>(resolve => { finishLoad = resolve; });
    const revision = reader.getLoadRevision();
    void notifyReading(book.id, { chapterId: chapter.id, chapterIndex: index, offset: position, updatedAtMs: Date.now() })
      .catch(error => { if (reader.getLoadRevision() === revision && reader.book.value?.id === book.id) failPending(actions.errorText(error)); });
    request.value += 1;
    return done;
  }
  async function startReading(book: BookResource, index?: number, offset?: number): Promise<void> {
    await actions.showShelfBehindReader(book);
    reader.visible.value = true;
    reader.controlsOpen.value = false;
    reader.directoryOpen.value = false;
    reader.controlsMode.value = "settings";
    reader.menuOpen.value = !!book.mediaType;
    reader.pdfZoomOverride.value = null;
    reader.resetClickSuppression();
    await loadReaderChapter(index ?? book.progress?.chapterIndex ?? 0, offset ?? (index == null ? book.progress?.offset ?? 0 : 0));
  }
  async function reloadCurrentChapter(): Promise<void> {
    const book = reader.book.value;
    const chapter = book?.chapters.find(entry => entry.index === reader.chapterIndex.value);
    if (!book || !chapter || reader.mediaRetrying.value) return;
    if (!(await actions.pauseReaderMediaAndSave())) return;
    reader.mediaRetrying.value = true;
    try {
      await notifyReading(book.id, { chapterId: chapter.id, chapterIndex: chapter.index, offset: reader.mediaChapter.value ? reader.mediaTimeMs.value : reader.imagePage.value ? reader.comicScrollTop.value : reader.pageIndex.value, updatedAtMs: Date.now() }, true);
      if (reader.visible.value && reader.book.value?.id === book.id && reader.chapterIndex.value === chapter.index) {
        request.value += 1;
      }
    } catch (error) { reader.mediaError.value = actions.errorText(error); actions.notify(reader.mediaError.value, "error"); }
    finally { reader.mediaRetrying.value = false; }
  }
  return { startReading, loadReaderChapter, reloadCurrentChapter, retryReaderMedia: reloadCurrentChapter, failPending, cancel: () => complete(false) };
}
