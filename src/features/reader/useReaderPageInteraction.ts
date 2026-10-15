import { openedBook } from "../books/bookDetailState";
import type { Ref } from "vue";
import { settings } from "../settings/settingsStore";
import type { PdfPageResource } from "./readerContent";

type ReadonlyRef<T> = Readonly<Ref<T>>;

export interface ReaderTouchStartState {
  x: number;
  y: number;
  moved: boolean;
  loadRevision: number;
  viewport: HTMLElement;
}

export interface ReaderPendingFragmentState {
  fragment: string;
  loadRevision: number;
  chapterIndex: number;
}

interface ReaderPageInteractionOptions {
  reader: {
    viewport: ReadonlyRef<HTMLElement | null>;
    chapterIndex: ReadonlyRef<number>;
    pageIndex: ReadonlyRef<number>;
    pageCount: ReadonlyRef<number>;
    visible: ReadonlyRef<boolean>;
    busy: ReadonlyRef<boolean>;
    mediaChapter: ReadonlyRef<boolean>;
    imagePage: ReadonlyRef<boolean>;
    pdfPage: ReadonlyRef<PdfPageResource | null>;
    sourceRefreshBusy: ReadonlyRef<boolean>;
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
  actions: {
    setPage: (index: number) => void;
    turnPage: (direction: -1 | 1) => void;
    loadChapter: (index: number, requestedMediaOffsetMs?: number, requestedFragment?: string) => Promise<boolean>;
  };
}

/** 管理阅读正文的触控、点击、章节内跳转和本章查找交互。 */
export function useReaderPageInteraction(options: ReaderPageInteractionOptions) {
  const { reader, actions } = options;

  function resolveSemanticChapterAnchor(anchor: HTMLAnchorElement): { chapterIndex: number; fragment: string } | null {
    const chapterId = anchor.dataset.legadoChapterId?.trim();
    if (!chapterId) return null;
    const book = openedBook.value;
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
    const book = openedBook.value;
    const currentSrc = openedBook.value?.chapters.find(chapter => chapter.index === reader.chapterIndex.value)?.resource?.src;
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

  function readerSegmentForElement(target: Element): number {
    return readerSegmentForRect(target.ownerDocument, target.getBoundingClientRect());
  }

  function readerSegmentForRect(_document: Document, rect: DOMRect): number {
    const viewport = reader.viewport.value;
    if (viewport) {
      const bounds = viewport.getBoundingClientRect();
      return settings.value.reader.readingMode === "vertical"
        ? Math.floor(Math.max(0, rect.top - bounds.top + viewport.scrollTop) / Math.max(1, viewport.clientHeight))
        : Math.floor(Math.max(0, rect.left - bounds.left + viewport.scrollLeft) / Math.max(1, viewport.clientWidth));
    }
    return 0;
  }

  function scrollReaderToFragment(fragment: string, loadRevision: number, chapterIndex: number): boolean {
    const frame = reader.viewport.value;
    const document = frame?.ownerDocument;
    if (!document || frame !== reader.viewport.value || loadRevision !== reader.getLoadRevision() || chapterIndex !== reader.chapterIndex.value
      || reader.imagePage.value || reader.mediaChapter.value || reader.pdfPage.value) return false;
    const target = frame?.querySelector(`[id="${CSS.escape(fragment)}"],[name="${CSS.escape(fragment)}"]`);
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
    if (reader.busy.value) return;
    reader.setPendingFragment(null);
    if (pending.fragment) scrollReaderToFragment(pending.fragment, pending.loadRevision, pending.chapterIndex);
    else actions.setPage(0);
  }

  function onReaderFrameTouchCancel(): void {
    reader.setTouchStart(null);
  }

  function onReaderFrameClick(event: MouseEvent): void {
    const document = reader.viewport.value?.ownerDocument;
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
    const frameBounds = reader.viewport.value?.getBoundingClientRect();
    if (!frameBounds) return;
    if (reader.viewport.value) event.stopPropagation();
    handleReaderTapAt(event.clientX);
  }

  function handleReaderTapAt(x: number): void {
    if (reader.busy.value || reader.sourceRefreshBusy.value) return;
    if (reader.menuOpen.value) {
      reader.menuOpen.value = false;
      reader.controlsOpen.value = false;
      reader.directoryOpen.value = false;
      return;
    }
    const width = window.innerWidth;
    if (x < width / 3) {
      // In text mode, the first line can itself sit inside the left tap zone.
      // Keep that tap on the chapter instead of treating it as a chapter jump.
      // A deliberate swipe can still cross the chapter boundary.
      if (reader.imagePage.value || reader.pdfPage.value) actions.turnPage(-1);
      else if (reader.pageIndex.value > 0) actions.turnPage(-1);
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
    if (reader.mediaChapter.value) return;
    if (Date.now() < reader.getClickSuppressedUntil()) return;
    const node = event.target as Node | null;
    const target = node?.nodeType === Node.ELEMENT_NODE ? node as Element : node?.parentElement;
    if (!target?.closest(".reader-main")
      || target.closest("button,input,select,textarea,[role='button'],.reader-edge,.reader-settings-popover,.reader-topbar,.reader-footer")) return;
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
    const root = reader.viewport.value?.querySelector<HTMLElement>(".chapter-content");
    const document = root?.ownerDocument;
    if (!root || !document) {
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
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
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
  };
}
