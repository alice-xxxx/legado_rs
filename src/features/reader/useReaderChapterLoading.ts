import { nextTick, type Ref } from "vue";
import { prepareChapters } from "../../api/books";
import { readResource } from "../../api/resources";
import { type AppSettingsResource, type BookResource, type ChapterMediaResource, type ChapterResourceDescriptor, type ChapterPdfPageResource, type ChapterRichTextResource, type ChapterTextResource, type DisplayReplacementRule } from "../../api/types";
import {
  hasDisplayableReaderContent,
  isImageOnlyChapter,
  makeDisplayHtml,
  mediaResourceToDisplayHtml,
  plainTextToDisplayHtml,
  richTextResourceToDisplayHtml,
  typedPdfPageResource,
} from "./displayHtml";
import type { PdfPageResource } from "./displayHtml";
import type { ReaderPendingFragmentState } from "./useReaderPageInteraction";

type ReadonlyRef<T> = Readonly<Ref<T>>;

export interface ReaderChapterLoadingOptions {
  reader: {
    book: Ref<BookResource | null>;
    openedBook: Ref<BookResource | null>;
    chapterIndex: Ref<number>;
    chapterHtml: Ref<string>;
    chapterRaw: Ref<string>;
    pdfPage: Ref<PdfPageResource | null>;
    pdfZoomOverride: Ref<PdfPageResource["defaultZoom"] | null>;
    pdfInitialPassword: Ref<string>;
    imagePage: Ref<boolean>;
    mediaChapter: Ref<boolean>;
    videoChapter: Ref<boolean>;
    mediaTimeMs: Ref<number>;
    mediaDurationMs: Ref<number>;
    mediaError: Ref<string>;
    mediaRetrying: Ref<boolean>;
    pageIndex: Ref<number>;
    pageCount: Ref<number>;
    comicScrollTop: Ref<number>;
    comicScaleMode: ReadonlyRef<"fit-width" | "actual-size">;
    visible: Ref<boolean>;
    controlsOpen: Ref<boolean>;
    directoryOpen: Ref<boolean>;
    controlsMode: Ref<"settings" | "tts" | "appearance" | "search">;
    menuOpen: Ref<boolean>;
    busy: Ref<boolean>;
    progressDirty: Ref<boolean>;
    progressSaveState: Ref<"idle" | "saving" | "saved" | "error">;
    prefetchBusy: Ref<boolean>;
    shelfBatchRecoveryRequired: ReadonlyRef<boolean>;
    pdfImportPasswords: Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>;
    settings: ReadonlyRef<AppSettingsResource>;
    replacementRules: ReadonlyRef<DisplayReplacementRule[]>;
    currentChapterTitle: ReadonlyRef<string>;
    getMediaElement: () => HTMLMediaElement | null;
    getLoadRevision: () => number;
    advanceLoadRevision: () => number;
    advanceProgressRevision: () => void;
    getTtsPlayback: () => string;
    getTtsExpectedChapterKey: () => string | null;
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
    scrollReaderFrameToPage: () => void;
    measureReaderPages: () => void;
    beginReadingSession: () => void;
    isBackupRestoreInProgress: () => boolean;
    notify: (message: string, kind?: "success" | "error") => void;
    errorText: (error: unknown) => string;
  };
}

/** 管理阅读器章节准备、读取与媒体重试。 */
export function useReaderChapterLoading(options: ReaderChapterLoadingOptions) {
  const { reader, actions } = options;
  let preparePromise: Promise<void> | null = null;
  let prepareOperations = 0;
  let startReadingRequest = 0;

  function beginPreparing(): void {
    prepareOperations += 1;
    reader.prefetchBusy.value = true;
  }

  function finishPreparing(): void {
    prepareOperations = Math.max(0, prepareOperations - 1);
    reader.prefetchBusy.value = prepareOperations > 0;
  }

  // 旧数据或不完整响应可能缺少目录；阅读器内部统一使用有效数组。
  function normalizeReaderBook(book: BookResource): BookResource {
    const chapters = Array.isArray(book.chapters) ? book.chapters : [];
    return {
      ...book,
      chapterCount: Number.isFinite(book.chapterCount) ? Math.max(0, book.chapterCount) : chapters.length,
      chapters,
    };
  }

  function preparedChapterResource(
    chapter: BookResource["chapters"][number] | undefined,
  ): ChapterResourceDescriptor | null {
    return chapter?.resource ?? null;
  }

  async function ensureChapterResource(bookId: string, index: number, loadRevision: number): Promise<BookResource> {
    const isCurrent = () => reader.visible.value
      && reader.book.value?.id === bookId
      && reader.getLoadRevision() === loadRevision
      && !actions.isBackupRestoreInProgress();
    if (!isCurrent()) throw new Error("当前章节请求已失效。");

    // Serialize explicit preparation, without letting an old rejected request
    // prevent preparing the current book.
    if (preparePromise) {
      try {
        await preparePromise;
      } catch {
        // The originating chapter request handles its own failure.
      }
    }
    if (!isCurrent()) throw new Error("当前章节请求已失效。");

    beginPreparing();
    const pending = (async () => {
      const response = await prepareChapters(bookId, index, 1);
      const updatedBook = normalizeReaderBook(await readResource<BookResource>(response.book));
      if (!isCurrent()) return;
      if (updatedBook.id !== bookId) throw new Error("章节准备返回了不匹配的书籍。");
      reader.book.value = updatedBook;
      if (reader.openedBook.value?.id === bookId) reader.openedBook.value = updatedBook;
    })();
    preparePromise = pending;
    try {
      await pending;
    } finally {
      if (preparePromise === pending) preparePromise = null;
      finishPreparing();
    }

    if (!isCurrent()) throw new Error("当前章节请求已失效。");
    const book = reader.book.value;
    const chapter = book?.chapters.find((entry) => entry.index === index);
    if (!book || !preparedChapterResource(chapter)) {
      throw new Error("章节准备失败，请返回目录后重试。");
    }
    return book;
  }

  async function requestUpcomingChapters(book: BookResource, fromIndex: number): Promise<void> {
    const total = Math.max(1, book.chapterCount || book.chapters.length);
    const wanted = Math.min(reader.settings.value.reader.preloadCount, total - fromIndex);
    if (fromIndex >= total || wanted < 1 || reader.prefetchBusy.value) return;
    const loadRevision = reader.getLoadRevision();
    const isCurrent = () => reader.visible.value
      && reader.book.value === book
      && reader.getLoadRevision() === loadRevision
      && !actions.isBackupRestoreInProgress();

    // Rust decides cache validity; prefetch may refresh only the same reader
    // snapshot it started from, never replace a newer foreground chapter load.
    beginPreparing();
    try {
      const response = await prepareChapters(book.id, fromIndex, wanted);
      const updated = normalizeReaderBook(await readResource<BookResource>(response.book));
      if (!isCurrent() || updated.id !== book.id) return;
      reader.book.value = updated;
      if (reader.openedBook.value?.id === book.id) reader.openedBook.value = updated;
    } catch (error) {
      if (isCurrent()) actions.notify(`后续章节缓存失败：${actions.errorText(error)}`, "error");
    } finally {
      finishPreparing();
    }
  }

  async function readPreparedChapter(
    book: BookResource,
    index: number,
  ): Promise<{
    book: BookResource;
    html: string;
    pdfPage?: PdfPageResource;
  }> {
    const chapter = book.chapters.find((entry) => entry.index === index);
    if (!chapter) {
      throw new Error(`第 ${index + 1} 章没有可读取的资源地址。`);
    }
    const chapterResource = preparedChapterResource(chapter);
    const contentFormat = chapterResource?.contentFormat;
    if (contentFormat === "text") {
      if (!chapterResource) throw new Error("正文资源缺少描述信息，请重新获取本章。");
      const resource = await readResource<ChapterTextResource>(chapterResource);
      if (resource.kind !== "text" || typeof resource.text !== "string") {
        throw new Error("正文资源格式无效，请重新获取本章。");
      }
      return { book, html: plainTextToDisplayHtml(resource.text) };
    }

    if (contentFormat === "richText") {
      if (!chapterResource) throw new Error("富文本资源缺少描述信息，请重新获取本章。");
      const resource = await readResource<ChapterRichTextResource>(chapterResource);
      return { book, html: richTextResourceToDisplayHtml(resource) };
    }

    if (contentFormat === "audio" || contentFormat === "video") {
      if (!chapterResource) throw new Error("媒体资源缺少描述信息，请重新获取本章。");
      const resource = await readResource<ChapterMediaResource>(chapterResource);
      if (resource.kind !== "media" || resource.mediaType !== contentFormat) {
        throw new Error("媒体章节资源格式无效，请重新获取本章。");
      }
      return { book, html: mediaResourceToDisplayHtml(resource) };
    }

    if (contentFormat === "pdf") {
      if (!chapterResource) throw new Error("PDF 资源缺少描述信息，请重新导入本书。");
      const resource = await readResource<ChapterPdfPageResource>(chapterResource);
      const pdfPage = typedPdfPageResource(resource);
      if (!pdfPage) throw new Error("PDF 页面资源格式无效，请重新导入本书。");
      return { book, html: "", pdfPage };
    }

    throw new Error("章节资源格式不受支持，请重新获取本章。");
  }

  async function startReading(book: BookResource, requestedIndex?: number, requestedOffset?: number): Promise<void> {
    const request = ++startReadingRequest;
    const openingBook = normalizeReaderBook(book);
    await actions.showShelfBehindReader(openingBook);
    if (request !== startReadingRequest) return;
    actions.stopTts(false);
    if (reader.getMediaElement()) {
      const audioProgressSaved = await actions.pauseReaderMediaAndSave();
      if (request !== startReadingRequest) return;
      if (!audioProgressSaved) {
        actions.notify("媒体播放位置尚未保存，请稍后再打开其他章节。", "error");
        return;
      }
      actions.clearReaderMediaBinding(false);
    }
    reader.pdfZoomOverride.value = null;
    const savedIndex = openingBook.progress?.chapterIndex ?? 0;
    const chapterCount = Math.max(openingBook.chapterCount, openingBook.chapters.length);
    const firstIndex = Math.max(0, Math.min(Math.max(0, chapterCount - 1), requestedIndex ?? savedIndex));
    const mediaBook = openingBook.mediaType === "audio" || openingBook.mediaType === "video";
    reader.videoChapter.value = openingBook.mediaType === "video";
    reader.mediaTimeMs.value = mediaBook
      ? Math.max(0, requestedOffset ?? (openingBook.progress?.chapterIndex === firstIndex ? openingBook.progress.offset ?? 0 : 0))
      : 0;
    reader.mediaDurationMs.value = 0;
    reader.mediaError.value = "";
    reader.book.value = openingBook;
    reader.chapterError.value = "";
    reader.chapterIndex.value = firstIndex;
    reader.pageIndex.value = mediaBook ? 0 : requestedOffset != null
      ? Math.max(0, requestedOffset)
      : requestedIndex == null && openingBook.progress?.chapterIndex === firstIndex
        ? Math.max(0, openingBook.progress.offset ?? 0)
        : 0;
    reader.visible.value = true;
    reader.controlsOpen.value = false;
    reader.directoryOpen.value = false;
    reader.controlsMode.value = "settings";
    reader.menuOpen.value = mediaBook;
    reader.resetClickSuppression();
    reader.progressDirty.value = false;
    reader.progressSaveState.value = "idle";
    const loaded = await loadReaderChapter(firstIndex, mediaBook ? requestedOffset : undefined);
    if (!loaded || request !== startReadingRequest || !reader.visible.value || reader.book.value?.id !== openingBook.id) return;
    if (requestedOffset != null && reader.chapterRaw.value) {
      if (reader.mediaChapter.value) {
        reader.mediaTimeMs.value = Math.max(0, requestedOffset);
      } else if (reader.imagePage.value) {
        reader.comicScrollTop.value = Math.max(0, requestedOffset);
        reader.pageIndex.value = reader.comicScrollTop.value;
      } else {
        reader.pageIndex.value = Math.min(Math.max(0, requestedOffset), reader.pageCount.value - 1);
      }
      if (!reader.mediaChapter.value) {
        reader.progressDirty.value = true;
        reader.advanceProgressRevision();
        if (reader.imagePage.value) actions.applyComicScrollPosition();
        else actions.scrollReaderFrameToPage();
      }
    }
    if (reader.chapterRaw.value) actions.beginReadingSession();
  }

  async function loadReaderChapter(index: number, requestedMediaOffsetMs?: number, requestedFragment?: string): Promise<boolean> {
    if (!reader.book.value || actions.isBackupRestoreInProgress()) return false;
    const targetTtsChapterKey = `${reader.book.value.id}:${index}`;
    reader.chapterError.value = "";
    reader.errorChapterIndex.value = null;
    if (reader.getTtsPlayback() !== "idle" && reader.getTtsExpectedChapterKey() !== targetTtsChapterKey) actions.stopTts(false);
    reader.setPendingFragment(null);
    const bookId = reader.book.value.id;
    if (reader.getMediaElement()) {
      const previousLoadRevision = reader.getLoadRevision();
      const mediaProgressSaved = await actions.pauseReaderMediaAndSave();
      if (reader.getLoadRevision() !== previousLoadRevision || reader.book.value?.id !== bookId || !mediaProgressSaved) return false;
      actions.clearReaderMediaBinding(false);
    }
    const loadRevision = reader.advanceLoadRevision();
    reader.chapterHtml.value = "";
    reader.chapterRaw.value = "";
    reader.pdfPage.value = null;
    reader.imagePage.value = false;
    reader.mediaChapter.value = false;
    reader.videoChapter.value = false;
    reader.pdfInitialPassword.value = "";
    reader.mediaError.value = "";
    actions.clearReaderFindMatches();
    actions.clearReaderScrollListeners();
    reader.busy.value = true;
    let loadedSuccessfully = false;
    try {
      const preparedBook = await ensureChapterResource(bookId, index, loadRevision);
      const loaded = await readPreparedChapter(preparedBook, index);
      if (loadRevision !== reader.getLoadRevision() || actions.isBackupRestoreInProgress() || !reader.visible.value || reader.book.value?.id !== bookId) return false;
      const { book, html: rawHtml, pdfPage: typedPdfPage } = loaded;
      const pdfPage = typedPdfPage ?? null;
      const imageOnly = !pdfPage && isImageOnlyChapter(rawHtml);
      const audioOnly = !pdfPage && book.mediaType === "audio";
      const videoOnly = !pdfPage && book.mediaType === "video";
      const mediaOnly = audioOnly || videoOnly;
      reader.chapterIndex.value = index;
      reader.chapterRaw.value = rawHtml;
      reader.pdfPage.value = pdfPage;
      const importedPassword = reader.pdfImportPasswords.get(book.id);
      reader.pdfInitialPassword.value = pdfPage && importedPassword && importedPassword.timer ? importedPassword.password : "";
      reader.imagePage.value = imageOnly;
      reader.mediaChapter.value = mediaOnly;
      reader.videoChapter.value = videoOnly;
      const savedOffset = book.progress?.chapterIndex === index ? Math.max(0, book.progress.offset ?? 0) : 0;
      reader.pageIndex.value = mediaOnly ? 0 : savedOffset;
      reader.comicScrollTop.value = imageOnly ? savedOffset : 0;
      reader.mediaTimeMs.value = mediaOnly ? Math.max(0, requestedMediaOffsetMs ?? savedOffset) : 0;
      reader.mediaDurationMs.value = 0;
      reader.mediaError.value = "";
      reader.setPendingFragment(requestedFragment != null && !pdfPage && !imageOnly && !mediaOnly
        ? { fragment: requestedFragment, loadRevision, chapterIndex: index, frameLoaded: false }
        : null);
      const displayHtml = pdfPage
        ? ""
        : makeDisplayHtml(rawHtml, reader.settings.value.reader, reader.replacementRules.value, book.id, {
          imageOnly,
          audioOnly,
          videoOnly,
          comicScaleMode: reader.comicScaleMode.value,
          chapterTitle: reader.currentChapterTitle.value,
        }, reader.visible.value);
      if (!pdfPage && !hasDisplayableReaderContent(displayHtml)) {
        throw new Error("正文处理后没有可显示的内容。可以重试，或从书源重新获取本章。");
      }
      reader.chapterHtml.value = displayHtml;
      reader.errorChapterIndex.value = null;
      reader.pageCount.value = 1;
      reader.progressDirty.value = true;
      reader.advanceProgressRevision();
      reader.progressSaveState.value = "idle";
      await nextTick();
      if (imageOnly) actions.applyComicScrollPosition();
      else if (!pdfPage && !mediaOnly) actions.measureReaderPages();
      if (index + 1 < Math.max(book.chapterCount, book.chapters.length)) void requestUpcomingChapters(book, index + 1);
      loadedSuccessfully = true;
    } catch (error) {
      if (loadRevision !== reader.getLoadRevision() || actions.isBackupRestoreInProgress() || !reader.visible.value || reader.book.value?.id !== bookId) return false;
      reader.setPendingFragment(null);
      reader.chapterHtml.value = "";
      reader.menuOpen.value = true;
      reader.errorChapterIndex.value = index;
      reader.chapterError.value = `无法打开章节：${actions.errorText(error)}`;
      actions.notify("章节资源读取失败。", "error");
    } finally {
      if (loadRevision === reader.getLoadRevision()) {
        reader.busy.value = false;
        actions.applyPendingReaderFragment();
      }
    }
    return loadedSuccessfully;
  }

  async function retryReaderMedia(): Promise<void> {
    const book = reader.book.value;
    if (!book || !reader.mediaChapter.value || reader.busy.value || reader.mediaRetrying.value
      || actions.isBackupRestoreInProgress() || reader.shelfBatchRecoveryRequired.value) return;
    const bookId = book.id;
    const chapterIndex = reader.chapterIndex.value;
    const loadRevision = reader.getLoadRevision();
    const isCurrentRetry = () => reader.visible.value
      && reader.book.value?.id === bookId
      && reader.chapterIndex.value === chapterIndex
      && reader.getLoadRevision() === loadRevision
      && !actions.isBackupRestoreInProgress()
      && !reader.shelfBatchRecoveryRequired.value;
    const originalMediaElement = reader.getMediaElement();
    const hadControls = originalMediaElement?.controls ?? false;
    reader.mediaRetrying.value = true;
    reader.busy.value = true;
    if (originalMediaElement) originalMediaElement.controls = false;
    try {
      if (!(await actions.pauseReaderMediaAndSave())) {
        throw new Error("当前播放位置尚未保存，请稍后重试。");
      }
      if (!isCurrentRetry()) return;
      const offsetMs = reader.mediaTimeMs.value;
      const response = await prepareChapters(bookId, chapterIndex, 1);
      const refreshedBook = await readResource<BookResource>(response.book);
      if (!isCurrentRetry()) return;
      reader.book.value = refreshedBook;
      if (reader.openedBook.value?.id === bookId) reader.openedBook.value = refreshedBook;
      reader.mediaTimeMs.value = offsetMs;

      // 保留旧章节画面，书源刷新失败时仍可从当前位置继续重试。
      reader.busy.value = false;
      const loaded = await loadReaderChapter(chapterIndex, offsetMs);
      if (!loaded && reader.visible.value && reader.book.value?.id === bookId && reader.chapterIndex.value === chapterIndex) {
        reader.mediaError.value = `重新加载${reader.videoChapter.value ? "视频" : "音频"}失败，请重试。`;
        actions.notify(reader.mediaError.value, "error");
      }
    } catch (error) {
      if (isCurrentRetry()) {
        reader.mediaError.value = `重新加载${reader.videoChapter.value ? "视频" : "音频"}失败：${actions.errorText(error)}`;
        actions.notify(reader.mediaError.value, "error");
      }
    } finally {
      reader.mediaRetrying.value = false;
      if (reader.getLoadRevision() === loadRevision) reader.busy.value = false;
      if (originalMediaElement && reader.getMediaElement() === originalMediaElement) originalMediaElement.controls = hadControls;
    }
  }

  return { startReading, loadReaderChapter, retryReaderMedia };
}
