import type { Ref } from "vue";
import { settings } from "../settings/settingsStore";
import type { ReaderTouchStartState } from "./useReaderPageInteraction";

type ReadonlyRef<T> = Readonly<Ref<T>>;
export function useReaderDisplay(options: {
  reader: {
    viewport: Ref<HTMLElement | null>;
    visible: ReadonlyRef<boolean>; busy: ReadonlyRef<boolean>;
    imagePage: ReadonlyRef<boolean>; mediaChapter: ReadonlyRef<boolean>;
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
    onReaderFrameTouchTap: (x: number) => void; onReaderFrameTouchCancel: () => void;
  };
  actions: {
    setPage: (index: number) => void; turnPage: (direction: -1 | 1) => void;
    saveProgress: () => Promise<void>; markProgress: () => void;
    stopTts: () => void; clearTtsHighlight: () => void; updateTtsSelection: () => void;
  };
}) {
  const { reader, actions } = options;
  function measureReaderPages(preserveRatio = false): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.imagePage.value || reader.mediaChapter.value) return;
    const previousCount = reader.pageCount.value;
    const previousIndex = reader.pageIndex.value;
    const vertical = settings.value.reader.verticalScroll;
    const size = Math.max(1, vertical ? viewport.clientHeight : viewport.clientWidth);
    reader.pageCount.value = Math.max(1, Math.ceil(((vertical ? viewport.scrollHeight : viewport.scrollWidth) - 1) / size));
    reader.pageIndex.value = preserveRatio && previousCount > 1
      ? Math.round(reader.pageIndex.value / (previousCount - 1) * (reader.pageCount.value - 1))
      : Math.min(reader.pageIndex.value, reader.pageCount.value - 1);
    scrollReaderFrameToPage();
    if (previousIndex !== reader.pageIndex.value) { actions.markProgress(); void actions.saveProgress(); }
  }
  function scrollReaderFrameToPage(): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.imagePage.value || reader.mediaChapter.value) return;
    const vertical = settings.value.reader.verticalScroll;
    viewport.scrollTo({ left: vertical ? 0 : reader.pageIndex.value * viewport.clientWidth,
      top: vertical ? reader.pageIndex.value * viewport.clientHeight : 0, behavior: "instant" as ScrollBehavior });
  }
  function onReaderFrameScroll(): void {
    const viewport = reader.viewport.value;
    if (!viewport || !reader.visible.value || reader.busy.value) return;
    if (reader.imagePage.value) {
      const offset = Math.max(0, Math.round(viewport.scrollTop));
      if (offset === reader.comicScrollTop.value) return;
      reader.comicScrollTop.value = offset;
    } else {
      const vertical = settings.value.reader.verticalScroll;
      const index = Math.min(reader.pageCount.value - 1, Math.max(0, Math.round(
        vertical ? viewport.scrollTop / Math.max(1, viewport.clientHeight) : viewport.scrollLeft / Math.max(1, viewport.clientWidth))));
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
  function onReaderFrameTouchStart(event: TouchEvent): void {
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
    if (event.touches.length !== 1) { reader.setTouchStart(null); return; }
    if (!start || start.viewport.ownerDocument.getSelection()?.isCollapsed === false) return;
    const touch = event.touches[0]!;
    const dx = Math.abs(touch.clientX - start.x), dy = Math.abs(touch.clientY - start.y);
    if (dx >= 12 || dy >= 12) start.moved = true;
    if (!reader.imagePage.value && !settings.value.reader.verticalScroll && dx >= 12 && dx > dy && event.cancelable) event.preventDefault();
  }
  function onReaderFrameTouchEnd(event: TouchEvent): void {
    const start = reader.getTouchStart(); reader.setTouchStart(null);
    if (!start || event.changedTouches.length !== 1 || event.touches.length !== 0 || reader.busy.value
      || reader.viewport.value !== start.viewport || reader.getLoadRevision() !== start.loadRevision
      || start.viewport.ownerDocument.getSelection()?.isCollapsed === false || ignoredTouch(event.target)) return;
    const touch = event.changedTouches[0]!;
    const dx = touch.clientX - start.x, dy = touch.clientY - start.y;
    if (!start.moved && Math.abs(dx) < 12 && Math.abs(dy) < 12) {
      if (!reader.imagePage.value || touch.clientX >= window.innerWidth / 3 && touch.clientX <= window.innerWidth * 2 / 3) {
        if (event.cancelable) event.preventDefault();
        reader.setClickSuppressedUntil(Date.now() + 500);
        options.getPageInteraction().onReaderFrameTouchTap(touch.clientX);
      }
      return;
    }
    reader.setClickSuppressedUntil(Date.now() + 500);
    if (reader.imagePage.value || settings.value.reader.verticalScroll) return;
    if (event.cancelable) event.preventDefault();
    if (Math.abs(dx) >= 48 && Math.abs(dy) <= 52 && Math.abs(dx) >= Math.abs(dy) * 1.35) actions.turnPage(dx < 0 ? 1 : -1);
    else scrollReaderFrameToPage();
  }
  function applyComicScrollPosition(): void {
    if (reader.imagePage.value && reader.viewport.value) reader.viewport.value.scrollTop = reader.comicScrollTop.value;
  }
  function clearReaderScrollListeners(): void {
    const element = reader.viewport.value;
    const interaction = options.getPageInteraction();
    element?.ownerDocument.removeEventListener("selectionchange", actions.updateTtsSelection);
    element?.removeEventListener("scroll", onReaderFrameScroll);
    element?.removeEventListener("touchstart", onReaderFrameTouchStart);
    element?.removeEventListener("touchmove", onReaderFrameTouchMove);
    element?.removeEventListener("touchend", onReaderFrameTouchEnd);
    element?.removeEventListener("touchcancel", interaction.onReaderFrameTouchCancel);
    element?.removeEventListener("click", interaction.onReaderFrameClick);
    reader.setTouchStart(null);
  }
  function setReaderViewport(element: HTMLElement | null): void {
    clearReaderScrollListeners();
    reader.viewport.value = element;
    if (!element) return;
    element.addEventListener("scroll", onReaderFrameScroll, { passive: true });
    element.addEventListener("touchstart", onReaderFrameTouchStart, { passive: true });
    element.addEventListener("touchmove", onReaderFrameTouchMove, { passive: false });
    element.addEventListener("touchend", onReaderFrameTouchEnd, { passive: false });
    element.addEventListener("touchcancel", options.getPageInteraction().onReaderFrameTouchCancel, { passive: true });
    if (!reader.imagePage.value) element.addEventListener("click", options.getPageInteraction().onReaderFrameClick);
    element.ownerDocument.addEventListener("selectionchange", actions.updateTtsSelection);
  }
  function onReaderTextChange(): void {
    actions.stopTts(); actions.clearTtsHighlight(); options.getPageInteraction().clearReaderFindMatches();
  }
  function onReaderLayout(): void { measureReaderPages(true); options.getPageInteraction().rebuildReaderFind(); }
  return { clearReaderScrollListeners, setReaderViewport, onReaderTextChange, onReaderLayout, measureReaderPages,
    scrollReaderFrameToPage, onReaderFrameScroll, shouldIgnoreReaderKeydown,
    onReaderFrameTouchStart, onReaderFrameTouchMove, onReaderFrameTouchEnd, applyComicScrollPosition };
}
