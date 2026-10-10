import type { Ref } from "vue";
import type { BookResource, ReaderSettings } from "../../api/types";
import type { PdfPageResource } from "./displayHtml";

type ReadonlyRef<T> = Readonly<Ref<T>>;

export interface ReaderTouchStartState {
  x: number;
  y: number;
  pageIndex: number;
  loadRevision: number;
  chapterIndex: number;
  frame: HTMLIFrameElement;
  document: Document;
}

export interface ReaderPendingFragmentState {
  fragment: string;
  loadRevision: number;
  chapterIndex: number;
  frameLoaded: boolean;
}

interface ReaderPageInteractionOptions {
  reader: {
    frame: ReadonlyRef<HTMLIFrameElement | null>;
    book: ReadonlyRef<BookResource | null>;
    currentChapterSource: ReadonlyRef<string | null | undefined>;
    chapterIndex: ReadonlyRef<number>;
    pageCount: ReadonlyRef<number>;
    visible: ReadonlyRef<boolean>;
    busy: ReadonlyRef<boolean>;
    mediaChapter: ReadonlyRef<boolean>;
    imagePage: ReadonlyRef<boolean>;
    pdfPage: ReadonlyRef<PdfPageResource | null>;
    sourceRefreshBusy: ReadonlyRef<boolean>;
    settings: ReadonlyRef<ReaderSettings>;
    menuOpen: Ref<boolean>;
    controlsOpen: Ref<boolean>;
    directoryOpen: Ref<boolean>;
    findQuery: ReadonlyRef<string>;
    findMatches: Ref<HTMLElement[]>;
    findIndex: Ref<number>;
    getLoadRevision: () => number;
    getPendingFragment: () => ReaderPendingFragmentState | null;
    setPendingFragment: (value: ReaderPendingFragmentState | null) => void;
    getTouchStart: () => ReaderTouchStartState | null;
    setTouchStart: (value: ReaderTouchStartState | null) => void;
    getClickSuppressedUntil: () => number;
    setClickSuppressedUntil: (value: number) => void;
  };
  frameBinding: {
    getDocument: () => Document | null;
    setDocument: (value: Document | null) => void;
    getWindow: () => Window | null;
    setWindow: (value: Window | null) => void;
    getScrollElement: () => HTMLElement | null;
    setScrollElement: (value: HTMLElement | null) => void;
  };
  events: {
    onScroll: () => void;
    onSelectionChange: () => void;
    onKeydown: (event: KeyboardEvent) => void;
    onTouchStart: (event: TouchEvent) => void;
    onTouchMove: (event: TouchEvent) => void;
    onTouchEnd: (event: TouchEvent) => void;
  };
  actions: {
    setPage: (index: number) => void;
    turnPage: (direction: -1 | 1) => void;
    loadChapter: (index: number, requestedMediaOffsetMs?: number, requestedFragment?: string) => Promise<boolean>;
  };
}

/** 管理阅读 iframe 的触控、点击、章节内跳转和本章查找交互。 */
export function useReaderPageInteraction(options: ReaderPageInteractionOptions) {
  const { reader, frameBinding, events, actions } = options;

  function resolveSemanticChapterAnchor(anchor: HTMLAnchorElement): { chapterIndex: number; fragment: string } | null {
    const chapterId = anchor.dataset.legadoChapterId?.trim();
    if (!chapterId) return null;
    const book = reader.book.value;
    if (!book) return null;
    const chapter = book.chapters.find((entry) => entry.id === chapterId);
    const chapterCount = book.chapterCount || book.chapters.length;
    if (!chapter || !Number.isInteger(chapter.index) || chapter.index < 0 || chapter.index >= chapterCount) {
      return null;
    }
    const encodedFragment = anchor.dataset.legadoFragment?.trim() ?? "";
    try {
      return {
        chapterIndex: chapter.index,
        fragment: encodedFragment ? decodeURIComponent(encodedFragment) : "",
      };
    } catch {
      return null;
    }
  }

  function resolveReaderChapterHref(href: string): { chapterIndex: number; fragment: string } | null {
    const book = reader.book.value;
    const currentSrc = reader.currentChapterSource.value;
    if (!book || !currentSrc) return null;
    let currentUrl: URL;
    let targetUrl: URL;
    try {
      currentUrl = new URL(currentSrc);
      targetUrl = new URL(href, currentUrl);
    } catch {
      return null;
    }
    if (targetUrl.origin !== currentUrl.origin || targetUrl.username || targetUrl.password) return null;
    let fragment: string;
    try {
      fragment = decodeURIComponent(targetUrl.hash.slice(1));
    } catch {
      return null;
    }
    targetUrl.hash = "";
    const targetHref = targetUrl.href;
    const chapter = book.chapters.find((entry) => {
      const chapterSrc = entry.resource?.src;
      if (!chapterSrc || !Number.isInteger(entry.index) || entry.index < 0) return false;
      try {
        const chapterUrl = new URL(chapterSrc, currentUrl);
        chapterUrl.hash = "";
        return chapterUrl.origin === currentUrl.origin && chapterUrl.href === targetHref;
      } catch {
        return false;
      }
    });
    const chapterCount = book.chapterCount || book.chapters.length;
    if (!chapter || chapter.index >= chapterCount) return null;
    return { chapterIndex: chapter.index, fragment };
  }

  function findReaderFragmentTarget(document: Document, fragment: string): Element | null {
    try {
      const idTarget = document.getElementById(fragment);
      if (idTarget) return idTarget;
      return document.getElementsByName(fragment).item(0);
    } catch {
      return null;
    }
  }

  function readerSegmentForElement(target: Element): number {
    return readerSegmentForRect(target.ownerDocument, target.getBoundingClientRect());
  }

  function readerSegmentForRect(document: Document, rect: DOMRect): number {
    if (reader.settings.value.verticalScroll) {
      const height = Math.max(1, document.documentElement.clientHeight || reader.frame.value?.clientHeight || 1);
      const scrollTop = Math.max(document.body.scrollTop, document.documentElement.scrollTop);
      const top = rect.top + scrollTop;
      return Math.floor(Math.max(0, top) / height);
    }
    const width = Math.max(1, document.documentElement.clientWidth || reader.frame.value?.clientWidth || 1);
    const scrollLeft = Math.max(document.body.scrollLeft, document.documentElement.scrollLeft);
    const left = rect.left + scrollLeft;
    return Math.floor(Math.max(0, left) / width);
  }

  function scrollReaderToFragment(fragment: string, loadRevision: number, chapterIndex: number): boolean {
    const frame = reader.frame.value;
    const document = frame?.contentDocument;
    if (!document || frame !== reader.frame.value || loadRevision !== reader.getLoadRevision() || chapterIndex !== reader.chapterIndex.value
      || reader.imagePage.value || reader.mediaChapter.value || reader.pdfPage.value) return false;
    const target = findReaderFragmentTarget(document, fragment);
    if (!target) return false;
    actions.setPage(readerSegmentForElement(target));
    return true;
  }

  function applyPendingReaderFragment(): void {
    const pending = reader.getPendingFragment();
    if (!pending) return;
    if (pending.loadRevision !== reader.getLoadRevision() || pending.chapterIndex !== reader.chapterIndex.value || !reader.visible.value) {
      reader.setPendingFragment(null);
      return;
    }
    if (reader.busy.value || !pending.frameLoaded) return;
    reader.setPendingFragment(null);
    if (pending.fragment) scrollReaderToFragment(pending.fragment, pending.loadRevision, pending.chapterIndex);
    else actions.setPage(0);
  }

  function onReaderFrameTouchCancel(): void {
    reader.setTouchStart(null);
  }

  function onReaderFrameClick(event: MouseEvent): void {
    const document = frameBinding.getDocument();
    if (!document || Date.now() < reader.getClickSuppressedUntil() || event.defaultPrevented || event.button !== 0
      || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    if (!reader.visible.value || reader.busy.value || reader.mediaChapter.value
      || document.getSelection()?.isCollapsed === false) return;
    const node = event.target as (Node & { closest?: (selectors: string) => Element | null }) | null;
    const element = node?.nodeType === Node.ELEMENT_NODE ? node as Element : node?.parentElement;
    const anchor = element?.closest<HTMLAnchorElement>("a[href],a[data-legado-chapter-id]");
    if (anchor) {
      if (anchor.hasAttribute("download")) return;
      const target = anchor.getAttribute("target")?.trim().toLocaleLowerCase("en-US") ?? "";
      if (target && target !== "_self") return;
      const destination = resolveSemanticChapterAnchor(anchor)
        ?? resolveReaderChapterHref(anchor.getAttribute("href") ?? anchor.href);
      if (!destination) return;
      if (destination.chapterIndex === reader.chapterIndex.value) {
        event.preventDefault();
        if (destination.fragment) scrollReaderToFragment(destination.fragment, reader.getLoadRevision(), destination.chapterIndex);
        else actions.setPage(0);
        return;
      }
      event.preventDefault();
      void actions.loadChapter(destination.chapterIndex, undefined, destination.fragment);
      return;
    }
    if (element?.closest("button,input,select,textarea,[role='button'],[contenteditable='true']")) return;
    const frameBounds = reader.frame.value?.getBoundingClientRect();
    if (!frameBounds) return;
    handleReaderTapAt(frameBounds.left + event.clientX);
  }

  function handleReaderTapAt(x: number): void {
    if (reader.busy.value || reader.sourceRefreshBusy.value) return;
    const width = window.innerWidth;
    if (x < width / 3) {
      actions.turnPage(-1);
      return;
    }
    if (x > width * 2 / 3) {
      actions.turnPage(1);
      return;
    }
    reader.menuOpen.value = !reader.menuOpen.value;
    if (!reader.menuOpen.value) {
      reader.controlsOpen.value = false;
      reader.directoryOpen.value = false;
    }
  }

  function onReaderFrameTouchTap(x: number): void {
    if (!reader.visible.value || reader.busy.value || reader.mediaChapter.value || reader.pdfPage.value) return;
    handleReaderTapAt(x);
  }

  function onReaderSurfaceClick(event: MouseEvent): void {
    if (Date.now() < reader.getClickSuppressedUntil()) return;
    const node = event.target as Node | null;
    const target = node?.nodeType === Node.ELEMENT_NODE ? node as Element : node?.parentElement;
    if (!target?.closest(".reader-main")
      || target.closest("button,input,select,textarea,[role='button'],.reader-edge,.reader-settings-popover,.reader-topbar,.reader-footer,.reader-video-quality-control")) return;
    handleReaderTapAt(event.clientX);
  }

  function clearReaderFindMatches(): void {
    for (const mark of reader.findMatches.value) {
      const parent = mark.parentNode;
      if (!parent) continue;
      mark.replaceWith(mark.ownerDocument.createTextNode(mark.textContent ?? ""));
      parent.normalize();
    }
    reader.findMatches.value = [];
    reader.findIndex.value = -1;
  }

  function rebuildReaderFind(visitFirstMatch = false): void {
    const document = reader.frame.value?.contentDocument;
    if (!document?.body) {
      clearReaderFindMatches();
      return;
    }
    const selection = document.getSelection();
    const selectedRanges = selection
      ? Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index).cloneRange())
      : [];
    clearReaderFindMatches();
    const query = reader.findQuery.value.trim().toLowerCase();
    if (!query) {
      if (selection) {
        selection.removeAllRanges();
        selectedRanges.forEach((range) => selection.addRange(range));
      }
      return;
    }

    const occurrences: Array<{ node: Text; start: number; end: number }> = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const parent = node.parentElement;
        return parent && !parent.closest("script,style,noscript,textarea,input,select,button,svg,canvas,video,audio,object,iframe,[contenteditable]:not([contenteditable='false'])")
          ? NodeFilter.FILTER_ACCEPT
          : NodeFilter.FILTER_REJECT;
      },
    });
    while (walker.nextNode()) {
      const node = walker.currentNode as Text;
      const text = node.data.toLowerCase();
      for (let start = text.indexOf(query); start >= 0; start = text.indexOf(query, start + query.length)) {
        occurrences.push({ node, start, end: start + query.length });
      }
    }

    const matches: HTMLElement[] = [];
    for (const occurrence of occurrences.reverse()) {
      try {
        const range = document.createRange();
        range.setStart(occurrence.node, occurrence.start);
        range.setEnd(occurrence.node, occurrence.end);
        const mark = document.createElement("mark");
        mark.dataset.readerFindMatch = "true";
        mark.style.backgroundColor = "#fff176";
        mark.style.color = "#27251f";
        mark.style.borderRadius = "2px";
        range.surroundContents(mark);
        matches.push(mark);
      } catch {
        // 小写转换可能扩展字符长度，使原始文本节点中的偏移量失效。
      }
    }
    reader.findMatches.value = matches.reverse();
    reader.findIndex.value = reader.findMatches.value.length ? 0 : -1;
    updateReaderFindHighlight();
    if (selection) {
      selection.removeAllRanges();
      selectedRanges.forEach((range) => selection.addRange(range));
    }
    if (visitFirstMatch && reader.findMatches.value.length) focusReaderFindMatch(0);
  }

  function updateReaderFindHighlight(): void {
    reader.findMatches.value.forEach((mark, index) => {
      const current = index === reader.findIndex.value;
      mark.dataset.readerFindCurrent = String(current);
      mark.style.backgroundColor = current ? "#ffb74d" : "#fff176";
    });
  }

  function focusReaderFindMatch(index: number): void {
    const mark = reader.findMatches.value[index];
    if (!mark) return;
    actions.setPage(readerSegmentForElement(mark));
  }

  function moveReaderFind(direction: -1 | 1): void {
    const count = reader.findMatches.value.length;
    if (!count) return;
    const nextIndex = (reader.findIndex.value + direction + count) % count;
    reader.findIndex.value = nextIndex;
    updateReaderFindHighlight();
    focusReaderFindMatch(nextIndex);
  }

  function handleReaderFindKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter" || event.isComposing) return;
    event.preventDefault();
    moveReaderFind(event.shiftKey ? -1 : 1);
  }

  function clearReaderScrollListeners(): void {
    const document = frameBinding.getDocument();
    document?.removeEventListener("scroll", events.onScroll, true);
    document?.removeEventListener("selectionchange", events.onSelectionChange);
    document?.removeEventListener("keydown", events.onKeydown);
    document?.removeEventListener("touchstart", events.onTouchStart);
    document?.removeEventListener("touchmove", events.onTouchMove);
    document?.removeEventListener("touchend", events.onTouchEnd);
    document?.removeEventListener("touchcancel", onReaderFrameTouchCancel);
    document?.removeEventListener("click", onReaderFrameClick);
    frameBinding.getWindow()?.removeEventListener("scroll", events.onScroll);
    frameBinding.getScrollElement()?.removeEventListener("scroll", events.onScroll);
    reader.setTouchStart(null);
    frameBinding.setDocument(null);
    frameBinding.setWindow(null);
    frameBinding.setScrollElement(null);
  }

  return {
    onReaderFrameTouchCancel,
    readerSegmentForElement,
    readerSegmentForRect,
    scrollReaderToFragment,
    applyPendingReaderFragment,
    onReaderFrameClick,
    onReaderFrameTouchTap,
    onReaderSurfaceClick,
    clearReaderFindMatches,
    rebuildReaderFind,
    moveReaderFind,
    handleReaderFindKeydown,
    clearReaderScrollListeners,
  };
}
