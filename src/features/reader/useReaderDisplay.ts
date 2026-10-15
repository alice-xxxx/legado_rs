import { imagePosition, textPosition, restoreImagePosition, restoreTextPosition, type ReaderPosition } from "./readerPosition";
import { onScopeDispose, type Ref } from "vue";
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
  let imageResume: ReaderPosition | null = null;
  let lastPosition: ReaderPosition | null = null;
  let viewportWidth = 0;
  let viewportHeight = 0;
  let activePageTurnOverlay: HTMLElement | null = null;
  let pageTurnTimer: ReturnType<typeof setTimeout> | undefined;
  function clearPageTurnAnimation(): void {
    if (pageTurnTimer !== undefined) clearTimeout(pageTurnTimer);
    pageTurnTimer = undefined;
    activePageTurnOverlay?.remove();
    activePageTurnOverlay = null;
  }
  onScopeDispose(clearPageTurnAnimation);
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
    const viewport = reader.viewport.value;
    if (!viewport || reader.busy.value || reader.mediaChapter.value) return;
    const position = lastPosition;
    viewportWidth = viewport.clientWidth;
    viewportHeight = viewport.clientHeight;
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
    const size = Math.max(1, vertical ? viewport.clientHeight : viewport.clientWidth);
    reader.pageCount.value = Math.max(1, Math.ceil(((vertical ? viewport.scrollHeight : viewport.scrollWidth) - 1) / size));
    reader.pageIndex.value = preserveRatio && previousCount > 1
      ? Math.round(reader.pageIndex.value / (previousCount - 1) * (reader.pageCount.value - 1))
      : Math.min(reader.pageIndex.value, reader.pageCount.value - 1);
    scrollReaderFrameToPage();
    if (previousIndex !== reader.pageIndex.value) { actions.markProgress(); void actions.saveProgress(); }
  }
  function animatePageTurn(viewport: HTMLElement, direction: -1 | 1): boolean {
    const mode = settings.value.reader.readingMode ?? "cover";
    const frame = viewport.parentElement;
    if (mode === "none" || mode === "vertical" || !frame || !frame.classList.contains("reader-content-frame")
      || viewport.clientWidth <= 0 || viewport.clientHeight <= 0
      || viewport.ownerDocument.defaultView?.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
    const animation: "cover" | "simulation" = mode;

    clearPageTurnAnimation();
    const overlay = viewport.cloneNode(true) as HTMLElement;
    overlay.classList.add("reader-page-turn-overlay", `reader-page-turn-${animation}`);
    overlay.setAttribute("aria-hidden", "true");
    overlay.setAttribute("inert", "");
    overlay.style.position = "absolute";
    overlay.style.inset = "0";
    overlay.style.zIndex = "5";
    overlay.style.margin = "0";
    overlay.style.pointerEvents = "none";
    overlay.style.scrollBehavior = "auto";
    overlay.scrollLeft = viewport.scrollLeft;
    overlay.scrollTop = viewport.scrollTop;
    overlay.querySelectorAll<HTMLElement>("*").forEach((node) => {
      node.removeAttribute("id");
      node.removeAttribute("contenteditable");
      for (const attribute of Array.from(node.attributes)) {
        if (attribute.name.toLowerCase().startsWith("on")) node.removeAttribute(attribute.name);
      }
      if (node.matches("a,button,input,select,textarea,[tabindex]")) node.setAttribute("tabindex", "-1");
    });
    overlay.querySelectorAll<HTMLMediaElement>("audio,video").forEach((media) => {
      media.pause();
      media.removeAttribute("autoplay");
      media.removeAttribute("src");
      media.querySelectorAll("source").forEach((source) => source.removeAttribute("src"));
      media.load();
    });

    const duration = animation === "cover" ? 360 : 460;
    const transform = (progress: number) => animation === "cover"
      ? `translateX(${direction > 0 ? -100 * progress : 100 * progress}%)`
      : `perspective(1200px) rotateY(${direction > 0 ? -78 : 78}deg)`;
    overlay.style.transformOrigin = direction > 0 ? "left center" : "right center";
    overlay.style.transform = animation === "cover" ? "translateX(0)" : "perspective(1200px) rotateY(0deg)";
    overlay.style.transition = "none";
    frame.append(overlay);
    activePageTurnOverlay = overlay;
    void overlay.offsetWidth;
    requestAnimationFrame(() => {
      if (activePageTurnOverlay !== overlay) return;
      overlay.style.transition = `transform ${duration}ms cubic-bezier(.2,.7,.2,1)`;
      overlay.classList.add("reader-page-turn-running");
      overlay.style.transform = transform(1);
    });
    pageTurnTimer = setTimeout(() => {
      if (activePageTurnOverlay === overlay) clearPageTurnAnimation();
    }, duration + 120);
    return true;
  }
  function scrollReaderFrameToPage(animate = false, direction: -1 | 1 = 1): void {
    const viewport = reader.viewport.value;
    if (!viewport || reader.imagePage.value || reader.mediaChapter.value) return;
    const vertical = settings.value.reader.readingMode === "vertical";
    const shouldAnimate = animate && !vertical && animatePageTurn(viewport, direction);
    viewport.scrollTo({ left: vertical ? 0 : reader.pageIndex.value * viewport.clientWidth,
      top: vertical ? reader.pageIndex.value * viewport.clientHeight : 0, behavior: "instant" as ScrollBehavior });
    if (!shouldAnimate) clearPageTurnAnimation();
  }
  function onReaderFrameScroll(): void {
    const viewport = reader.viewport.value;
    if (!viewport || !reader.visible.value || reader.busy.value) return;
    if (viewport.clientWidth !== viewportWidth || viewport.clientHeight !== viewportHeight) {
      handleReaderResize();
      return;
    }
    rememberPosition();
    if (reader.imagePage.value) {
      const offset = Math.max(0, Math.round(viewport.classList.contains("paged") ? viewport.scrollLeft : viewport.scrollTop));
      if (offset === reader.comicScrollTop.value) return;
      reader.comicScrollTop.value = offset;
    } else {
      const vertical = settings.value.reader.readingMode === "vertical";
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
    if (!reader.imagePage.value && settings.value.reader.readingMode !== "vertical" && dx >= 12 && dx > dy && event.cancelable) event.preventDefault();
  }
  function onReaderFrameTouchEnd(event: TouchEvent): void {
    const start = reader.getTouchStart(); reader.setTouchStart(null);
    if (!start || event.changedTouches.length !== 1 || event.touches.length !== 0 || reader.busy.value
      || reader.viewport.value !== start.viewport || reader.getLoadRevision() !== start.loadRevision
      || start.viewport.ownerDocument.getSelection()?.isCollapsed === false || ignoredTouch(event.target)) return;
    const touch = event.changedTouches[0]!;
    const dx = touch.clientX - start.x, dy = touch.clientY - start.y;
    if (!start.moved && Math.abs(dx) < 12 && Math.abs(dy) < 12) {
      reader.setClickSuppressedUntil(Date.now() + 500);
      if (reader.imagePage.value) return;
      if (event.cancelable) event.preventDefault();
      options.getPageInteraction().onReaderFrameTouchTap(touch.clientX);
      return;
    }
    reader.setClickSuppressedUntil(Date.now() + 500);
    if (reader.imagePage.value || settings.value.reader.readingMode === "vertical") return;
    if (event.cancelable) event.preventDefault();
    if (Math.abs(dx) >= 48 && Math.abs(dy) <= 52 && Math.abs(dx) >= Math.abs(dy) * 1.35) actions.turnPage(dx < 0 ? 1 : -1);
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
      reader.pageIndex.value = Math.max(0, Math.floor((vertical ? viewport.scrollTop : viewport.scrollLeft)
        / Math.max(1, vertical ? viewport.clientHeight : viewport.clientWidth)));
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
    lastPosition = null;
    viewportWidth = element?.clientWidth ?? 0;
    viewportHeight = element?.clientHeight ?? 0;
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
    clearPageTurnAnimation();
    actions.stopTts(); actions.clearTtsHighlight(); options.getPageInteraction().clearReaderFindMatches();
  }
  function onReaderLayout(): void { handleReaderResize(); options.getPageInteraction().rebuildReaderFind(); }
  return { restoreReadingPosition, handleReaderResize, clearReaderScrollListeners, setReaderViewport, onReaderTextChange, onReaderLayout, measureReaderPages,
    scrollReaderFrameToPage, onReaderFrameScroll, shouldIgnoreReaderKeydown,
    onReaderFrameTouchStart, onReaderFrameTouchMove, onReaderFrameTouchEnd, applyComicScrollPosition };
}
