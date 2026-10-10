import type { Ref } from "vue";
import type { AppSettingsResource, BookResource, DisplayReplacementRule } from "../../api/types";
import { clamp, makeDisplayHtml } from "./displayHtml";
import type { PdfPageResource } from "./displayHtml";
import type { useReaderMedia } from "./useReaderMedia";
import type { useReaderMediaBinding, ReaderVideoQualityChoice } from "./useReaderMediaBinding";
import type { ReaderPendingFragmentState, ReaderTouchStartState, useReaderPageInteraction } from "./useReaderPageInteraction";

type ReadonlyRef<T> = Readonly<Ref<T>>;

type ReaderPageInteraction = Pick<
  ReturnType<typeof useReaderPageInteraction>,
  | "applyPendingReaderFragment"
  | "clearReaderFindMatches"
  | "clearReaderScrollListeners"
  | "onReaderFrameClick"
  | "onReaderFrameTouchTap"
  | "onReaderFrameTouchCancel"
  | "rebuildReaderFind"
>;

type ReaderFrameBinding = {
  getDocument: () => Document | null;
  setDocument: (value: Document | null) => void;
  getWindow: () => Window | null;
  setWindow: (value: Window | null) => void;
  getScrollElement: () => HTMLElement | null;
  setScrollElement: (value: HTMLElement | null) => void;
};

type ReaderDisplayOptions = {
  reader: {
    frame: Ref<HTMLIFrameElement | null>;
    visible: ReadonlyRef<boolean>;
    busy: ReadonlyRef<boolean>;
    book: ReadonlyRef<BookResource | null>;
    chapterIndex: ReadonlyRef<number>;
    chapterTitle: ReadonlyRef<string>;
    rawHtml: ReadonlyRef<string>;
    html: Ref<string>;
    settings: ReadonlyRef<AppSettingsResource>;
    replacementRules: ReadonlyRef<DisplayReplacementRule[]>;
    imagePage: ReadonlyRef<boolean>;
    mediaChapter: ReadonlyRef<boolean>;
    videoChapter: ReadonlyRef<boolean>;
    pdfPage: ReadonlyRef<PdfPageResource | null>;
    pageIndex: Ref<number>;
    pageCount: Ref<number>;
    progressDirty: Ref<boolean>;
    progressSaveState: Ref<"idle" | "saving" | "saved" | "error">;
    comicScrollTop: Ref<number>;
    comicScaleMode: ReadonlyRef<"fit-width" | "actual-size">;
    mediaError: Ref<string>;
    videoQualityChoices: Ref<ReaderVideoQualityChoice[]>;
    videoQualityIndex: Ref<number>;
    getLoadRevision: () => number;
    getPendingFragment: () => ReaderPendingFragmentState | null;
    setPendingFragment: (value: ReaderPendingFragmentState | null) => void;
    getTouchStart: () => ReaderTouchStartState | null;
    setTouchStart: (value: ReaderTouchStartState | null) => void;
    getClickSuppressedUntil: () => number;
    setClickSuppressedUntil: (value: number) => void;
  };
  media: {
    binding: Pick<
      ReturnType<typeof useReaderMediaBinding>,
      | "bindReaderMediaElement"
      | "clearReaderMediaBinding"
      | "getBindingRevision"
      | "invalidateSource"
      | "loadReaderMediaSource"
    >;
    playback: Pick<ReturnType<typeof useReaderMedia>, "destroyHlsInstance" | "isHlsMediaSource">;
  };
  getPageInteraction: () => ReaderPageInteraction;
  actions: {
    setPage: (index: number) => void;
    turnPage: (direction: -1 | 1) => void;
    closeReader: () => Promise<void>;
    incrementProgressRevision: () => void;
    stopHttpTtsAudio: (clearSelection?: boolean) => void;
    clearTtsHighlight: () => void;
    updateTtsSelection: () => void;
    notify: (message: string, kind?: "success" | "error") => void;
    errorText: (error: unknown) => string;
  };
};

/** 管理章节 HTML 的排版、分页测量和 iframe 交互编排。 */
export function useReaderDisplay(options: ReaderDisplayOptions) {
  let boundDocument: Document | null = null;
  let boundWindow: Window | null = null;
  let boundScrollElement: HTMLElement | null = null;

  const frameBinding: ReaderFrameBinding = {
    getDocument: () => boundDocument,
    setDocument: (value) => { boundDocument = value; },
    getWindow: () => boundWindow,
    setWindow: (value) => { boundWindow = value; },
    getScrollElement: () => boundScrollElement,
    setScrollElement: (value) => { boundScrollElement = value; },
  };

  function rebuildReaderDisplay(): void {
    const { reader, actions } = options;
    actions.stopHttpTtsAudio(false);
    if (!reader.visible.value || !reader.rawHtml.value || reader.pdfPage.value) return;
    reader.html.value = makeDisplayHtml(
      reader.rawHtml.value,
      reader.settings.value.reader,
      reader.replacementRules.value,
      reader.book.value?.id,
      {
        imageOnly: reader.imagePage.value,
        audioOnly: reader.mediaChapter.value && !reader.videoChapter.value,
        videoOnly: reader.videoChapter.value,
        comicScaleMode: reader.comicScaleMode.value,
        chapterTitle: reader.chapterTitle.value,
      },
      reader.visible.value,
    );
  }

  function measureReaderPages(preserveRatio = false): void {
    const { reader, actions } = options;
    if (reader.imagePage.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    const frame = reader.frame.value;
    const document = frame?.contentDocument;
    if (!document) return;
    const width = Math.max(1, document.documentElement.clientWidth || frame?.clientWidth || 1);
    const previousCount = reader.pageCount.value;
    const previousIndex = reader.pageIndex.value;
    if (reader.settings.value.reader.verticalScroll) {
      const height = Math.max(1, document.documentElement.clientHeight || frame?.clientHeight || 1);
      const totalHeight = Math.max(height, document.body.scrollHeight, document.documentElement.scrollHeight);
      reader.pageCount.value = Math.max(1, Math.ceil((totalHeight - 1) / height));
    } else {
      const totalWidth = Math.max(width, document.body.scrollWidth, document.documentElement.scrollWidth);
      reader.pageCount.value = Math.max(1, Math.ceil((totalWidth - 1) / width));
    }
    const nextIndex = preserveRatio && previousCount > 1
      ? Math.round((previousIndex / (previousCount - 1)) * (reader.pageCount.value - 1))
      : Math.min(reader.pageIndex.value, reader.pageCount.value - 1);
    reader.pageIndex.value = nextIndex;
    if (nextIndex !== previousIndex) {
      reader.progressDirty.value = true;
      actions.incrementProgressRevision();
      reader.progressSaveState.value = "idle";
    }
    scrollReaderFrameToPage();
  }

  function scrollReaderFrameToPage(): void {
    const { reader } = options;
    if (reader.imagePage.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    const frame = reader.frame.value;
    const document = frame?.contentDocument;
    if (!document) return;
    if (reader.settings.value.reader.verticalScroll) {
      const height = Math.max(1, document.documentElement.clientHeight || frame?.clientHeight || 1);
      const scrollTop = reader.pageIndex.value * height;
      const scroller = document.scrollingElement as HTMLElement | null;
      (scroller ?? document.body).scrollTo({ top: scrollTop, behavior: "instant" as ScrollBehavior });
      return;
    }
    const width = Math.max(1, document.documentElement.clientWidth || frame?.clientWidth || 1);
    document.body.scrollTo({ left: reader.pageIndex.value * width, behavior: "instant" as ScrollBehavior });
    document.documentElement.scrollLeft = reader.pageIndex.value * width;
  }

  function onReaderFrameScroll(): void {
    const { reader, actions } = options;
    const scroller = frameBinding.getScrollElement();
    const document = frameBinding.getDocument();
    if (!scroller || !document) return;
    if (reader.imagePage.value) {
      const offset = Math.max(0, Math.round(scroller.scrollTop || document.documentElement.scrollTop));
      if (offset === reader.comicScrollTop.value) return;
      reader.comicScrollTop.value = offset;
      reader.pageIndex.value = offset;
    } else {
      if (reader.pdfPage.value) return;
      let index: number;
      if (reader.settings.value.reader.verticalScroll) {
        const height = Math.max(1, document.documentElement.clientHeight || reader.frame.value?.clientHeight || 1);
        const scrollTop = Math.max(scroller.scrollTop, document.body.scrollTop, document.documentElement.scrollTop);
        index = Math.min(reader.pageCount.value - 1, Math.max(0, Math.round(scrollTop / height)));
      } else {
        const width = Math.max(1, document.documentElement.clientWidth || reader.frame.value?.clientWidth || 1);
        const scrollLeft = Math.max(scroller.scrollLeft, document.body.scrollLeft, document.documentElement.scrollLeft);
        index = Math.min(reader.pageCount.value - 1, Math.max(0, Math.round(scrollLeft / width)));
      }
      if (index === reader.pageIndex.value) return;
      reader.pageIndex.value = index;
    }
    reader.progressDirty.value = true;
    actions.incrementProgressRevision();
    reader.progressSaveState.value = "idle";
  }

  function shouldIgnoreReaderKeydown(event: KeyboardEvent, document: Document): boolean {
    if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return true;
    const target = event.target as HTMLElement | null;
    if (target?.closest?.("input,textarea,select,button,a[href],audio,video,[contenteditable]:not([contenteditable='false']),[tabindex]:not([tabindex='-1'])")) return true;
    return document.getSelection()?.isCollapsed === false;
  }

  function onReaderFrameKeydown(event: KeyboardEvent): void {
    const { reader, actions } = options;
    const document = frameBinding.getDocument();
    if (!document || !reader.visible.value || reader.busy.value || reader.imagePage.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    if (shouldIgnoreReaderKeydown(event, document)) return;
    if (event.key === "ArrowRight" || event.key === "PageDown" || event.key === " ") {
      event.preventDefault();
      actions.turnPage(1);
    } else if (event.key === "ArrowLeft" || event.key === "PageUp") {
      event.preventDefault();
      actions.turnPage(-1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      void actions.closeReader();
    }
  }

  function isIgnoredReaderTouchTarget(target: EventTarget | null): boolean {
    const node = target as (Node & { closest?: (selectors: string) => Element | null }) | null;
    const element = node?.nodeType === Node.ELEMENT_NODE ? node as Element : node?.parentElement;
    return Boolean(element?.closest?.("a[href],button,input,textarea,select,audio,video,[contenteditable]:not([contenteditable='false']),[tabindex]:not([tabindex='-1'])"));
  }

  function onReaderFrameTouchStart(event: TouchEvent): void {
    const { reader } = options;
    reader.setTouchStart(null);
    if (!reader.visible.value || reader.busy.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    if (event.touches.length !== 1 || isIgnoredReaderTouchTarget(event.target)) return;
    const document = frameBinding.getDocument();
    const frame = reader.frame.value;
    if (!document || !frame || document.getSelection()?.isCollapsed === false) return;
    const touch = event.touches[0];
    reader.setTouchStart({
      x: touch.clientX,
      y: touch.clientY,
      pageIndex: reader.pageIndex.value,
      loadRevision: reader.getLoadRevision(),
      chapterIndex: reader.chapterIndex.value,
      frame,
      document,
    });
  }

  function onReaderFrameTouchMove(event: TouchEvent): void {
    if (event.touches.length !== 1) options.reader.setTouchStart(null);
  }

  function onReaderFrameTouchEnd(event: TouchEvent): void {
    const { reader, actions } = options;
    const start = reader.getTouchStart();
    reader.setTouchStart(null);
    if (!start || event.changedTouches.length !== 1 || event.touches.length !== 0) return;
    if (!reader.visible.value || reader.busy.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    if (reader.getLoadRevision() !== start.loadRevision || reader.chapterIndex.value !== start.chapterIndex
      || reader.frame.value !== start.frame || frameBinding.getDocument() !== start.document) return;
    if (start.document.getSelection()?.isCollapsed === false || isIgnoredReaderTouchTarget(event.target)) return;
    const touch = event.changedTouches[0];
    const deltaX = touch.clientX - start.x;
    const deltaY = touch.clientY - start.y;
    if (Math.abs(deltaX) < 12 && Math.abs(deltaY) < 12) {
      const frameBounds = start.frame.getBoundingClientRect();
      const x = frameBounds.left + touch.clientX;
      const width = window.innerWidth;
      if (!reader.imagePage.value || (x >= width / 3 && x <= width * 2 / 3)) {
        // The iframe's touch event does not bubble to the reader shell. Handle
        // reader taps here and suppress the synthetic click that follows.
        reader.setClickSuppressedUntil(Math.max(reader.getClickSuppressedUntil(), Date.now() + 120));
        options.getPageInteraction().onReaderFrameTouchTap(x);
      }
      return;
    }
    if (reader.imagePage.value || reader.settings.value.reader.verticalScroll) return;
    if (Math.abs(deltaX) < 72 || Math.abs(deltaY) > 52 || Math.abs(deltaX) < Math.abs(deltaY) * 1.35) return;
    const direction = deltaX < 0 ? 1 : -1;
    reader.setClickSuppressedUntil(Date.now() + 500);
    if (direction < 0 && start.pageIndex === 0) {
      actions.turnPage(-1);
      return;
    }
    if (direction > 0 && start.pageIndex >= reader.pageCount.value - 1) {
      actions.turnPage(1);
      return;
    }
    actions.setPage(clamp(start.pageIndex + direction, 0, reader.pageCount.value - 1));
    // 个别 WebView 会停在两列之间，手势结束后校正到目标页。
    scrollReaderFrameToPage();
  }

  function applyComicScrollPosition(): void {
    const document = options.reader.frame.value?.contentDocument;
    if (!document || !options.reader.imagePage.value) return;
    if (document.body) document.body.scrollTop = options.reader.comicScrollTop.value;
    document.documentElement.scrollTop = options.reader.comicScrollTop.value;
    if (document.scrollingElement) document.scrollingElement.scrollTop = options.reader.comicScrollTop.value;
  }

  function bindFrameScrollListeners(document: Document, scroller: HTMLElement): void {
    const { reader } = options;
    frameBinding.setScrollElement(scroller);
    frameBinding.setDocument(document);
    frameBinding.setWindow(document.defaultView);
    scroller.addEventListener("scroll", onReaderFrameScroll, { passive: true });
    document.addEventListener("scroll", onReaderFrameScroll, { capture: true, passive: true });
    document.addEventListener("touchstart", onReaderFrameTouchStart, { passive: true });
    document.addEventListener("touchmove", onReaderFrameTouchMove, { passive: true });
    document.addEventListener("touchend", onReaderFrameTouchEnd, { passive: true });
    const interaction = options.getPageInteraction();
    document.addEventListener("touchcancel", interaction.onReaderFrameTouchCancel, { passive: true });
    if (reader.imagePage.value) {
      document.defaultView?.addEventListener("scroll", onReaderFrameScroll, { passive: true });
      applyComicScrollPosition();
      return;
    }
    document.addEventListener("selectionchange", options.actions.updateTtsSelection);
    document.addEventListener("keydown", onReaderFrameKeydown);
    document.addEventListener("click", interaction.onReaderFrameClick);
    document.defaultView?.addEventListener("scroll", onReaderFrameScroll, { passive: true });
  }

  function onReaderFrameLoad(event: Event): void {
    const { reader, media, actions } = options;
    if (event.currentTarget !== reader.frame.value) return;
    actions.clearTtsHighlight();
    const interaction = options.getPageInteraction();
    interaction.clearReaderScrollListeners();
    if (reader.mediaChapter.value) {
      media.binding.clearReaderMediaBinding(true);
      const element = reader.frame.value?.contentDocument?.querySelector<HTMLMediaElement>("audio,video");
      if (element) {
        reader.mediaError.value = "";
        media.binding.bindReaderMediaElement(element);
        let selectedHlsSource = "";
        if (reader.videoChapter.value && element.tagName.toLowerCase() === "video") {
          const choices = Array.from(element.querySelectorAll<HTMLSourceElement>("source[data-legado-media-label]"))
            .map((source) => ({
              label: source.dataset.legadoMediaLabel?.trim() || "清晰度",
              src: source.getAttribute("src") ? source.src : "",
              isHls: media.playback.isHlsMediaSource(source),
              unavailableReason: source.dataset.legadoMediaUnavailable,
              selected: source.dataset.legadoMediaSelected === "true",
            }));
          reader.videoQualityChoices.value = choices;
          const selectedIndex = choices.findIndex((choice) => choice.selected);
          reader.videoQualityIndex.value = selectedIndex >= 0 ? selectedIndex : 0;
          const selectedChoice = selectedIndex >= 0 ? choices[selectedIndex] : choices[0];
          if (selectedChoice?.isHls) selectedHlsSource = selectedChoice.src;
        }
        if (!selectedHlsSource && reader.videoQualityChoices.value.length === 0) {
          const markedSource = Array.from(element.querySelectorAll<HTMLSourceElement>("source"))
            .find((source) => media.playback.isHlsMediaSource(source));
          if (markedSource) selectedHlsSource = markedSource.getAttribute("src") ? markedSource.src : "";
          else if (media.playback.isHlsMediaSource(element)) selectedHlsSource = element.getAttribute("src") ? element.src : "";
        }
        if (selectedHlsSource) {
          try {
            media.binding.loadReaderMediaSource(
              element,
              selectedHlsSource,
              true,
              media.binding.getBindingRevision(),
              reader.getLoadRevision(),
            );
          } catch (error) {
            media.binding.invalidateSource();
            media.playback.destroyHlsInstance();
            element.pause();
            reader.mediaError.value = `无法初始化 HLS 播放：${actions.errorText(error)}`;
            actions.notify(reader.mediaError.value, "error");
          }
        }
      } else {
        reader.videoQualityChoices.value = [];
        reader.videoQualityIndex.value = 0;
        reader.mediaError.value = `本章没有可播放的${reader.videoChapter.value ? "视频" : "音频"}轨道。可以点击“重新加载”重试。`;
        actions.notify(reader.mediaError.value, "error");
      }
      return;
    }
    if (reader.imagePage.value) {
      interaction.clearReaderFindMatches();
      const document = reader.frame.value?.contentDocument;
      const scroller = document?.scrollingElement as HTMLElement | null;
      if (document && scroller) bindFrameScrollListeners(document, scroller);
      return;
    }
    const pending = reader.getPendingFragment();
    if (pending?.loadRevision === reader.getLoadRevision() && pending.chapterIndex === reader.chapterIndex.value) {
      pending.frameLoaded = true;
    }
    measureReaderPages(true);
    const document = reader.frame.value?.contentDocument;
    const scroller = document
      ? reader.settings.value.reader.verticalScroll ? (document.scrollingElement as HTMLElement | null) ?? document.body : document.body
      : null;
    if (document && scroller) bindFrameScrollListeners(document, scroller);
    interaction.applyPendingReaderFragment();
    interaction.rebuildReaderFind();
  }

  return {
    frameBinding,
    rebuildReaderDisplay,
    measureReaderPages,
    scrollReaderFrameToPage,
    onReaderFrameScroll,
    onReaderFrameKeydown,
    shouldIgnoreReaderKeydown,
    onReaderFrameTouchStart,
    onReaderFrameTouchMove,
    onReaderFrameTouchEnd,
    applyComicScrollPosition,
    onReaderFrameLoad,
  };
}
