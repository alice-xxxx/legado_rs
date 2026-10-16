import { imagePosition, textPosition, textChapterBounds, readerPageWidth, restoreImagePosition, restoreTextPosition, type ReaderPosition } from "./readerPosition";
import { onScopeDispose, watch, type Ref } from "vue";
import { settings } from "../settings/settingsStore";
import { hasPageTurnAnimation } from "../settings/readerModes";
import type { ReaderTouchStartState } from "./useReaderPageInteraction";
import { createReaderPageTurn, type ReaderPageTurn } from "./readerPageTurn";

type ReadonlyRef<T> = Readonly<Ref<T>>;
export function useReaderDisplay(options: {
  reader: {
    viewport: Ref<HTMLElement | null>;
    visible: ReadonlyRef<boolean>; busy: ReadonlyRef<boolean>;
    imagePage: ReadonlyRef<boolean>; mediaChapter: ReadonlyRef<boolean>;
    menuOpen: ReadonlyRef<boolean>;
    pageIndex: Ref<number>; pageCount: Ref<number>; comicScrollTop: Ref<number>;
    getLoadRevision: () => number;
    getTouchStart: () => ReaderTouchStartState | null;
    setTouchStart: (value: ReaderTouchStartState | null) => void;
    getClickSuppressedUntil: () => number;
    setClickSuppressedUntil: (value: number) => void;
  };
  getPageInteraction: () => {
    clearReaderFindMatches: () => void;
    rebuildReaderFind: () => void; onReaderFrameClick: (event: MouseEvent) => void;
    onReaderFrameTouchTap: (x: number, y: number) => void;
  };
  actions: {
    setPage: (index: number) => void; turnPage: (direction: -1 | 1) => void;
    saveProgress: () => Promise<void>; markProgress: () => void;
    stopTts: () => void; clearTtsHighlight: () => void; updateTtsSelection: () => void;
    onTextScroll: () => void;
    chapterPreview: (direction: -1 | 1) => HTMLElement | null;
    commitChapterTurn: (direction: -1 | 1) => void;
  };
}) {
  const { reader, actions } = options;
  let imageResume: ReaderPosition | null = null;
  let lastPosition: ReaderPosition | null = null;
  let viewportWidth = 0;
  let viewportHeight = 0;
  let activePageTurn: ReaderPageTurn | null = null;
  let wheelTurnUntil = 0;
  let chapterTurnDone: ((committed: boolean) => void) | null = null;
  let measuredMode = settings.value.reader.readingMode;
  let pageDrag: { origin: number; last: number; direction: -1 | 1; cancelled: boolean; target: number; chapter: boolean } | null = null;
  function clearPageTurnAnimation(): void {
    activePageTurn?.dispose();
    activePageTurn = null;
    pageDrag = null;
    const done = chapterTurnDone; chapterTurnDone = null; done?.(false);
  }
  onScopeDispose(clearPageTurnAnimation);
  watch(() => [settings.value.reader.readingMode, reader.visible.value, reader.menuOpen.value], () => {
    clearPageTurnAnimation();
    reader.setTouchStart(null);
  }, { flush: "sync" });
  function rememberPosition(): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.mediaChapter.value) return;
    viewportWidth = viewport.clientWidth;
    viewportHeight = viewport.clientHeight;
    lastPosition = reader.imagePage.value ? imagePosition(viewport)
      : textPosition(viewport, settings.value.reader.readingMode === "vertical");
  }
  function handleReaderResize(): void {
    clearPageTurnAnimation();
    reader.setTouchStart(null);
    const viewport = reader.viewport.value;
    if (!viewport || reader.busy.value || reader.mediaChapter.value) return;
    const position = lastPosition;
    const mode = settings.value.reader.readingMode;
    const resized = viewportWidth !== viewport.clientWidth || viewportHeight !== viewport.clientHeight || measuredMode !== mode;
    viewportWidth = viewport.clientWidth;
    viewportHeight = viewport.clientHeight;
    measuredMode = mode;
    if (!reader.imagePage.value && mode === "vertical") {
      reader.pageCount.value = Math.max(1, Math.ceil((textChapterBounds(viewport).height - 1) / Math.max(1, viewport.clientHeight)));
      if (resized && position) restoreReadingPosition(position);
      else rememberPosition();
      return;
    }
    measureReaderPages(false);
    if (position) restoreReadingPosition(position);
    else rememberPosition();
  }
  function measureReaderPages(preserveRatio = false): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.imagePage.value || reader.mediaChapter.value) return;
    const previousCount = reader.pageCount.value;
    const previousIndex = reader.pageIndex.value;
    const vertical = settings.value.reader.readingMode === "vertical";
    const size = Math.max(1, vertical ? viewport.clientHeight : readerPageWidth(viewport));
    reader.pageCount.value = Math.max(1, Math.ceil(((vertical ? textChapterBounds(viewport).height : viewport.scrollWidth) - 1) / size));
    reader.pageIndex.value = preserveRatio && previousCount > 1
      ? Math.round(reader.pageIndex.value / (previousCount - 1) * (reader.pageCount.value - 1))
      : Math.min(reader.pageIndex.value, reader.pageCount.value - 1);
    scrollReaderFrameToPage();
    if (previousIndex !== reader.pageIndex.value) { actions.markProgress(); void actions.saveProgress(); }
  }
  function animatePageTurn(viewport: HTMLElement, direction: -1 | 1): boolean {
    const mode = settings.value.reader.readingMode ?? "cover";
    const frame = viewport.parentElement;
    if (!hasPageTurnAnimation(mode) || !frame || !frame.classList.contains("reader-content-frame")
      || viewport.clientWidth <= 0 || viewport.clientHeight <= 0
      || viewport.ownerDocument.defaultView?.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
    clearPageTurnAnimation();
    const turn = createReaderPageTurn(viewport, direction, mode, reader.pageIndex.value);
    if (!turn) return false;
    activePageTurn = turn;
    turn.settle(true, () => { if (activePageTurn === turn) clearPageTurnAnimation(); });
    return true;
  }
  function scrollReaderFrameToPage(animate = false, direction: -1 | 1 = 1): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.imagePage.value || reader.mediaChapter.value) return;
    const vertical = settings.value.reader.readingMode === "vertical";
    const shouldAnimate = animate && !vertical && animatePageTurn(viewport, direction);
    const chapter = textChapterBounds(viewport);
    viewport.scrollTo({ left: vertical ? 0 : reader.pageIndex.value * readerPageWidth(viewport),
      top: vertical ? chapter.top + Math.min(reader.pageIndex.value * viewport.clientHeight, Math.max(0, chapter.height - viewport.clientHeight)) : 0, behavior: "instant" as ScrollBehavior });
    if (!shouldAnimate) clearPageTurnAnimation();
  }
  function onReaderFrameScroll(): void {
    const viewport = reader.viewport.value;
    if (!viewport || !reader.visible.value || reader.busy.value) return;
    if (viewport.clientWidth !== viewportWidth || viewport.clientHeight !== viewportHeight) {
      handleReaderResize();
      return;
    }
    if (!reader.imagePage.value && settings.value.reader.readingMode === "vertical") actions.onTextScroll();
    rememberPosition();
    if (reader.imagePage.value) {
      const offset = Math.max(0, Math.round(viewport.classList.contains("paged") ? viewport.scrollLeft : viewport.scrollTop));
      if (offset === reader.comicScrollTop.value) return;
      reader.comicScrollTop.value = offset;
    } else {
      const vertical = settings.value.reader.readingMode === "vertical";
      const chapter = textChapterBounds(viewport);
      if (vertical) reader.pageCount.value = Math.max(1, Math.ceil((chapter.height - 1) / Math.max(1, viewport.clientHeight)));
      const atBottom = vertical && viewport.scrollTop + viewport.clientHeight >= chapter.top + chapter.height - 1;
      const index = atBottom ? reader.pageCount.value - 1 : Math.min(reader.pageCount.value - 1, Math.max(0, Math.round(
        vertical ? (viewport.scrollTop - chapter.top) / Math.max(1, viewport.clientHeight) : viewport.scrollLeft / readerPageWidth(viewport))));
      if (index === reader.pageIndex.value) return;
      reader.pageIndex.value = index;
    }
    actions.markProgress();
    void actions.saveProgress();
  }
  function shouldIgnoreReaderKeydown(event: KeyboardEvent, document: Document): boolean {
    if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return true;
    const target = event.target as HTMLElement | null;
    return !!target?.closest("input,textarea,select,button,a[href],audio,video,[contenteditable]:not([contenteditable='false'])")
      || document.getSelection()?.isCollapsed === false;
  }
  function ignoredTouch(target: EventTarget | null): boolean {
    const node = target as Node | null;
    const element = node?.nodeType === 1 ? node as Element : node?.parentElement;
    return !!element?.closest("a[href],button,input,textarea,select,audio,video,[contenteditable]:not([contenteditable='false'])");
  }
  function onReaderFrameWheel(event: WheelEvent): void {
    const viewport = reader.viewport.value;
    if (!viewport || !reader.visible.value || reader.busy.value || reader.menuOpen.value || reader.imagePage.value
      || reader.mediaChapter.value || ignoredTouch(event.target)
      || viewport.ownerDocument.getSelection()?.isCollapsed === false) return;
    const mode = settings.value.reader.readingMode;
    if (mode !== "vertical") {
      const vertical = mode === "slide-vertical";
      const distance = vertical ? event.deltaY : event.deltaX;
      const crossDistance = vertical ? event.deltaX : event.deltaY;
      if (!distance || Math.abs(distance) <= Math.abs(crossDistance) * 1.35) return;
      if (event.cancelable) event.preventDefault();
      const now = Date.now(), previous = wheelTurnUntil;
      wheelTurnUntil = now + 200;
      if (now >= previous && !activePageTurn) actions.turnPage(distance > 0 ? 1 : -1);
      return;
    }
  }
  function onReaderFrameTouchStart(event: TouchEvent): void {
    clearPageTurnAnimation();
    reader.setTouchStart(null);
    const viewport = reader.viewport.value;
    if (!viewport || reader.busy.value || event.touches.length !== 1 || ignoredTouch(event.target)
      || viewport.ownerDocument.getSelection()?.isCollapsed === false) return;
    const touch = event.touches[0]!;
    reader.setTouchStart({ x: touch.clientX, y: touch.clientY, moved: false,
      loadRevision: reader.getLoadRevision(), viewport });
  }
  function onReaderFrameTouchMove(event: TouchEvent): void {
    const start = reader.getTouchStart();
    if (event.touches.length !== 1 || !start || reader.busy.value || !reader.visible.value
      || start.viewport !== reader.viewport.value || start.loadRevision !== reader.getLoadRevision()
      || start.viewport.ownerDocument.getSelection()?.isCollapsed === false) {
      onReaderFrameTouchCancel();
      return;
    }
    const touch = event.touches[0]!;
    const dx = Math.abs(touch.clientX - start.x), dy = Math.abs(touch.clientY - start.y);
    if (dx >= 12 || dy >= 12) start.moved = true;
    if (reader.imagePage.value || settings.value.reader.readingMode === "vertical" || reader.menuOpen.value) return;
    const mode = settings.value.reader.readingMode;
    const verticalTurn = mode === "slide-vertical";
    const distance = verticalTurn ? dy : dx, crossDistance = verticalTurn ? dx : dy;
    if (pageDrag || (distance >= 12 && distance > crossDistance * 1.35)) {
      if (event.cancelable) event.preventDefault();
      if (!pageDrag && !reader.menuOpen.value) {
        const origin = verticalTurn ? start.y : start.x;
        const position = verticalTurn ? touch.clientY : touch.clientX;
        const direction = position < origin ? 1 : -1;
        const target = reader.pageIndex.value + direction;
        const crossing = target < 0 || target >= reader.pageCount.value;
        const preview = crossing ? actions.chapterPreview(direction) : null;
        if ((!crossing || preview) && hasPageTurnAnimation(mode)) {
          const bounds = start.viewport.getBoundingClientRect();
          const destination = preview ? direction > 0 ? 0 : Math.max(0, Math.ceil((preview.scrollWidth - 1) / readerPageWidth(preview)) - 1) : target;
          activePageTurn = createReaderPageTurn(start.viewport, direction, mode, destination,
            { x: start.x - bounds.left, y: start.y - bounds.top }, preview ?? start.viewport);
          if (activePageTurn) pageDrag = { origin: origin - direction * 12, last: position, direction, cancelled: false, target, chapter: crossing };
        }
      }
      updatePageDrag(touch.clientX, touch.clientY);
    }
  }
  function updatePageDrag(x: number, y: number): void {
    if (!pageDrag || !activePageTurn) return;
    const position = activePageTurn.axis === "y" ? y : x;
    const movement = (pageDrag.last - position) * pageDrag.direction;
    // Match Legado: reversing the last drag cancels the turn; moving forward resumes it.
    if (Math.abs(movement) > 1) pageDrag.cancelled = movement < 0;
    pageDrag.last = position;
    const bounds = reader.viewport.value?.getBoundingClientRect();
    activePageTurn.update((pageDrag.origin - position) * pageDrag.direction / activePageTurn.size,
      { x: x - (bounds?.left ?? 0), y: y - (bounds?.top ?? 0) });
  }
  function onReaderFrameTouchCancel(): void {
    reader.setTouchStart(null);
    reader.setClickSuppressedUntil(Date.now() + 500);
    const turn = activePageTurn;
    pageDrag = null;
    if (turn) turn.settle(false, () => { if (activePageTurn === turn) clearPageTurnAnimation(); });
  }
  function onReaderFrameTouchEnd(event: TouchEvent): void {
    const start = reader.getTouchStart(); reader.setTouchStart(null);
    if (!start || event.changedTouches.length !== 1 || event.touches.length !== 0 || reader.busy.value
      || reader.viewport.value !== start.viewport || reader.getLoadRevision() !== start.loadRevision
      || !reader.visible.value || start.viewport.ownerDocument.getSelection()?.isCollapsed === false || ignoredTouch(event.target)) {
      onReaderFrameTouchCancel();
      return;
    }
    const touch = event.changedTouches[0]!;
    const dx = touch.clientX - start.x, dy = touch.clientY - start.y;
    if (!start.moved && Math.abs(dx) < 12 && Math.abs(dy) < 12) {
      reader.setClickSuppressedUntil(Date.now() + 500);
      if (reader.imagePage.value) return;
      if (event.cancelable) event.preventDefault();
      options.getPageInteraction().onReaderFrameTouchTap(touch.clientX, touch.clientY);
      return;
    }
    reader.setClickSuppressedUntil(Date.now() + 500);
    if (reader.imagePage.value || reader.menuOpen.value) return;
    if (settings.value.reader.readingMode === "vertical") {
      return;
    }
    if (event.cancelable) event.preventDefault();
    if (pageDrag && activePageTurn) {
      updatePageDrag(touch.clientX, touch.clientY);
      const drag = pageDrag;
      const turn = activePageTurn;
      // The curl engine decides whether a released fold turns or springs back.
      const commit = settings.value.reader.readingMode === "simulation" || (!drag.cancelled && turn.progress > 0);
      pageDrag = null;
      turn.settle(commit, committed => {
        if (activePageTurn !== turn) return;
        clearPageTurnAnimation();
        if (committed && reader.viewport.value === start.viewport && reader.getLoadRevision() === start.loadRevision
          && reader.visible.value && !reader.busy.value) {
          if (drag.chapter) actions.commitChapterTurn(drag.direction);
          else actions.setPage(drag.target);
        }
      });
      return;
    }
    const verticalTurn = settings.value.reader.readingMode === "slide-vertical";
    const distance = verticalTurn ? dy : dx, crossDistance = verticalTurn ? dx : dy;
    if (Math.abs(distance) >= 48 && Math.abs(crossDistance) <= 52 && Math.abs(distance) >= Math.abs(crossDistance) * 1.35) actions.turnPage(distance < 0 ? 1 : -1);
    else scrollReaderFrameToPage();
  }
  function restoreReadingPosition(position: ReaderPosition): void {
    const viewport = reader.viewport.value;
    if (!viewport) return;
    lastPosition = { ...position };
    viewportWidth = viewport.clientWidth;
    viewportHeight = viewport.clientHeight;
    if (reader.imagePage.value) {
      imageResume = position;
      applyComicScrollPosition();
    } else {
      const vertical = settings.value.reader.readingMode === "vertical";
      restoreTextPosition(viewport, position, vertical);
      const chapter = textChapterBounds(viewport);
      const atBottom = vertical && viewport.scrollTop + viewport.clientHeight >= chapter.top + chapter.height - 1;
      reader.pageIndex.value = atBottom ? reader.pageCount.value - 1 : Math.max(0, vertical
        ? Math.round((viewport.scrollTop - chapter.top) / Math.max(1, viewport.clientHeight))
        : Math.floor((viewport.scrollLeft + 1) / readerPageWidth(viewport)));
    }
  }
  function applyComicScrollPosition(): void {
    const viewport = reader.viewport.value;
    if (!reader.imagePage.value || !viewport) return;
    if (imageResume) {
      const images = Array.from(viewport.querySelectorAll<HTMLImageElement>("img[data-page-id]"));
      const target = Math.max(0, images.findIndex(image => image.dataset.pageId === imageResume!.pageId));
      if (!images.length || images.slice(0, target + 1).some(image => !image.complete)) return;
      restoreImagePosition(viewport, imageResume);
      reader.comicScrollTop.value = viewport.classList.contains("paged") ? viewport.scrollLeft : viewport.scrollTop;
      imageResume = null;
    } else if (viewport.classList.contains("paged")) viewport.scrollLeft = reader.comicScrollTop.value;
    else viewport.scrollTop = reader.comicScrollTop.value;
  }
  function clearReaderScrollListeners(): void {
    clearPageTurnAnimation();
    const element = reader.viewport.value;
    const interaction = options.getPageInteraction();
    element?.ownerDocument.removeEventListener("selectionchange", actions.updateTtsSelection);
    element?.removeEventListener("scroll", onReaderFrameScroll);
    element?.removeEventListener("wheel", onReaderFrameWheel);
    element?.removeEventListener("touchstart", onReaderFrameTouchStart);
    element?.removeEventListener("touchmove", onReaderFrameTouchMove);
    element?.removeEventListener("touchend", onReaderFrameTouchEnd);
    element?.removeEventListener("touchcancel", onReaderFrameTouchCancel);
    element?.removeEventListener("click", interaction.onReaderFrameClick);
    reader.setTouchStart(null);
  }
  function setReaderViewport(element: HTMLElement | null): void {
    clearReaderScrollListeners();
    reader.viewport.value = element;
    lastPosition = null;
    viewportWidth = element?.clientWidth ?? 0;
    viewportHeight = element?.clientHeight ?? 0;
    if (!element) return;
    element.addEventListener("scroll", onReaderFrameScroll, { passive: true });
    element.addEventListener("wheel", onReaderFrameWheel, { passive: false });
    element.addEventListener("touchstart", onReaderFrameTouchStart, { passive: true });
    element.addEventListener("touchmove", onReaderFrameTouchMove, { passive: false });
    element.addEventListener("touchend", onReaderFrameTouchEnd, { passive: false });
    element.addEventListener("touchcancel", onReaderFrameTouchCancel, { passive: true });
    if (!reader.imagePage.value) element.addEventListener("click", options.getPageInteraction().onReaderFrameClick);
    element.ownerDocument.addEventListener("selectionchange", actions.updateTtsSelection);
  }
  function onReaderTextChange(): void {
    clearPageTurnAnimation();
    actions.stopTts(); actions.clearTtsHighlight(); options.getPageInteraction().clearReaderFindMatches();
  }
  function onReaderLayout(): void { handleReaderResize(); options.getPageInteraction().rebuildReaderFind(); }
  function animateChapterTurn(preview: HTMLElement, direction: -1 | 1, done: (committed: boolean) => void): void {
    clearPageTurnAnimation();
    const viewport = reader.viewport.value, mode = settings.value.reader.readingMode;
    if (!viewport || !hasPageTurnAnimation(mode)) { done(true); return; }
    const target = direction > 0 ? 0 : Math.max(0, Math.ceil((preview.scrollWidth - 1) / readerPageWidth(viewport)) - 1);
    const turn = createReaderPageTurn(viewport, direction, mode, target, undefined, preview);
    if (!turn) { done(true); return; }
    activePageTurn = turn; chapterTurnDone = done;
    turn.settle(true, committed => {
      if (activePageTurn !== turn) return;
      chapterTurnDone = null;
      done(committed);
      clearPageTurnAnimation();
    });
  }
  return { restoreReadingPosition, handleReaderResize, clearReaderScrollListeners, setReaderViewport, onReaderTextChange, onReaderLayout, measureReaderPages,
    scrollReaderFrameToPage, animateChapterTurn, onReaderFrameScroll, shouldIgnoreReaderKeydown,
    onReaderFrameTouchStart, onReaderFrameTouchMove, onReaderFrameTouchEnd, applyComicScrollPosition };
}
