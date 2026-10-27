import { nextTick, onScopeDispose, type Ref } from "vue";
import type { PdfPageResource } from "./readerContent";
import { textChapterElement } from "./readerPosition";
import { collectReaderTtsSegments, resolveTtsChunkRange, type ReaderTtsChunkTarget, type ReaderTtsSegment } from "./ttsText";

type ReadonlyRef<T> = Readonly<Ref<T>>;

interface UseReaderTtsActionsOptions {
  reader: {
    viewport: ReadonlyRef<HTMLElement | null>;
    busy: ReadonlyRef<boolean>;
    visible: ReadonlyRef<boolean>;
    mediaChapter: ReadonlyRef<boolean>;
    imagePage: ReadonlyRef<boolean>;
    pdfPage: ReadonlyRef<PdfPageResource | null>;
    getLoadRevision: () => number;
  };
  selection: {
    getCurrentText: () => string;
    rememberedText: ReadonlyRef<string>;
    error: Ref<string>;
  };
  playback: {
    start: (selectedText?: string) => Promise<void>;
  };
  display: {
    openControls: () => void;
    setPage: (index: number) => void;
    segmentForRect: (document: Document, rect: DOMRect) => number;
    scrollToPage: () => void;
  };
}

/** 管理朗读入口、正文分段和当前朗读位置高亮。 */
export function useReaderTtsActions(options: UseReaderTtsActionsOptions) {
  let disposeHighlight = () => {};
  onScopeDispose(clearTtsHighlight);

  function openTtsControls(): void {
    options.display.openControls();
  }

  function clearTtsHighlight(): void {
    disposeHighlight();
    disposeHighlight = () => {};
  }

  function highlightTtsChunk(target: ReaderTtsChunkTarget | undefined, loadRevision: number): void {
    clearTtsHighlight();
    const viewport = options.reader.viewport.value;
    const document = viewport?.ownerDocument;
    const view = document?.defaultView;
    const container = viewport?.parentElement;
    if (!target || !viewport || !document || !view || !container || !options.reader.visible.value
      || loadRevision !== options.reader.getLoadRevision() || options.reader.mediaChapter.value
      || options.reader.imagePage.value || options.reader.pdfPage.value) return;
    const range = resolveTtsChunkRange(textChapterElement(viewport), target);
    if (!range) return;
    // 与排版 DOM 分离，所有 WebView 都按实际文字矩形绘制，不扩大成整个段落。
    const namespace = "http://www.w3.org/2000/svg";
    const layer = document.createElementNS(namespace, "svg");
    layer.setAttribute("data-reader-tts-highlight", "true");
    layer.setAttribute("aria-hidden", "true");
    layer.style.cssText = "position:absolute;inset:0;width:100%;height:100%;overflow:hidden;pointer-events:none;z-index:2;opacity:.4;fill:#ffc400";
    container.appendChild(layer);
    let frame: number | null = null;
    const draw = () => {
      frame = null;
      if (options.reader.viewport.value !== viewport || !options.reader.visible.value || loadRevision !== options.reader.getLoadRevision()) {
        clearTtsHighlight();
        return;
      }
      layer.replaceChildren();
      const current = resolveTtsChunkRange(textChapterElement(viewport), target);
      if (!current) return;
      const bounds = container.getBoundingClientRect();
      const clip = viewport.getBoundingClientRect();
      layer.setAttribute("viewBox", `0 0 ${bounds.width} ${bounds.height}`);
      for (const rect of current.getClientRects()) {
        if (rect.width <= 0 || rect.height <= 0 || rect.right <= clip.left || rect.left >= clip.right || rect.bottom <= clip.top || rect.top >= clip.bottom) continue;
        const mark = document.createElementNS(namespace, "rect");
        mark.setAttribute("x", String(rect.left - bounds.left));
        mark.setAttribute("y", String(rect.top - bounds.top));
        mark.setAttribute("width", String(rect.width));
        mark.setAttribute("height", String(rect.height));
        layer.appendChild(mark);
      }
    };
    const schedule = () => { if (frame === null) frame = view.requestAnimationFrame(draw); };
    const resize = new view.ResizeObserver(schedule);
    const mutation = new view.MutationObserver(schedule);
    resize.observe(viewport);
    mutation.observe(viewport, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["class", "style"] });
    viewport.addEventListener("scroll", schedule, { passive: true });
    view.addEventListener("resize", schedule);
    disposeHighlight = () => {
      if (frame !== null) view.cancelAnimationFrame(frame);
      resize.disconnect();
      mutation.disconnect();
      viewport.removeEventListener("scroll", schedule);
      view.removeEventListener("resize", schedule);
      layer.remove();
    };
    const firstCharacter = range.cloneRange();
    firstCharacter.setEnd(range.startContainer, Math.min(range.startOffset + 1, (range.startContainer as Text).length));
    const rect = firstCharacter.getClientRects()[0];
    if (rect) {
      options.display.setPage(options.display.segmentForRect(document, rect));
      options.display.scrollToPage();
    }
    draw();
  }

  async function waitForReaderTtsSegments(fromCurrentPosition = false): Promise<ReaderTtsSegment[]> {
    await nextTick();
    const viewport = options.reader.viewport.value;
    if (!viewport) throw new Error("阅读画面尚未准备好。");
    return collectReaderTtsSegments(viewport.ownerDocument, fromCurrentPosition,
      { width: viewport.clientWidth, height: viewport.clientHeight }, textChapterElement(viewport));
  }

  function startSelectedTextTts(): void {
    const selected = options.selection.getCurrentText() || options.selection.rememberedText.value;
    if (!selected) {
      options.selection.error.value = "请先在章节正文中选中要朗读的文字。";
      return;
    }
    void options.playback.start(selected);
  }

  return {
    openTtsControls,
    clearTtsHighlight,
    highlightTtsChunk,
    waitForReaderTtsSegments,
    startSelectedTextTts,
  };
}
