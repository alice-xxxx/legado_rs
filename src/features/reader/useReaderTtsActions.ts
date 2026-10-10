import { nextTick, type Ref } from "vue";
import type { PdfPageResource } from "./displayHtml";
import { collectReaderTtsSegments, type ReaderTtsChunkTarget, type ReaderTtsSegment } from "./ttsText";

type ReadonlyRef<T> = Readonly<Ref<T>>;

interface UseReaderTtsActionsOptions {
  reader: {
    frame: ReadonlyRef<HTMLIFrameElement | null>;
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
    segmentForElement: (target: Element) => number;
    segmentForRect: (document: Document, rect: DOMRect) => number;
    scrollToPage: () => void;
  };
}

/** 管理朗读入口、正文分段和当前朗读位置高亮。 */
export function useReaderTtsActions(options: UseReaderTtsActionsOptions) {
  let ttsHighlightElement: HTMLElement | null = null;
  let ttsHighlightPreviousValue: string | null = null;
  let ttsHighlightStyle: HTMLStyleElement | null = null;
  let ttsHighlightDocument: Document | null = null;

  function openTtsControls(): void {
    options.display.openControls();
  }

  function clearTtsHighlight(): void {
    const document = ttsHighlightDocument;
    if (document) {
      const view = document.defaultView as unknown as { CSS?: { highlights?: { delete?: (name: string) => boolean } } } | null;
      try {
        view?.CSS?.highlights?.delete?.("legado-reader-tts-current");
      } catch {
        // 阅读 iframe 可能正在卸载，高亮清理失败时继续释放本地引用。
      }
    }
    if (ttsHighlightElement) {
      if (ttsHighlightPreviousValue === null) ttsHighlightElement.removeAttribute("data-legado-reader-tts-current");
      else ttsHighlightElement.setAttribute("data-legado-reader-tts-current", ttsHighlightPreviousValue);
    }
    ttsHighlightStyle?.remove();
    ttsHighlightDocument = null;
    ttsHighlightElement = null;
    ttsHighlightPreviousValue = null;
    ttsHighlightStyle = null;
  }

  function highlightTtsChunk(target: ReaderTtsChunkTarget | undefined, loadRevision: number): void {
    clearTtsHighlight();
    const frame = options.reader.frame.value;
    const document = target?.startNode?.ownerDocument ?? target?.target?.ownerDocument;
    if (!target || !document || document !== frame?.contentDocument || !options.reader.visible.value
      || loadRevision !== options.reader.getLoadRevision() || options.reader.mediaChapter.value
      || options.reader.imagePage.value || options.reader.pdfPage.value) return;

    let range: Range | null = null;
    if (target.startNode && target.endNode && target.startNode.isConnected && target.endNode.isConnected) {
      try {
        range = document.createRange();
        range.setStart(target.startNode, target.startOffset);
        range.setEnd(target.endNode, target.endOffset);
        if (range.collapsed) range = null;
      } catch {
        range = null;
      }
    }

    const view = document.defaultView as unknown as {
      CSS?: { highlights?: { set?: (name: string, value: unknown) => void } };
      Highlight?: new (...ranges: Range[]) => unknown;
    } | null;
    const registry = view?.CSS?.highlights;
    const HighlightConstructor = view?.Highlight;
    let highlightedWithRange = false;
    if (range && registry?.set && HighlightConstructor) {
      try {
        const highlight = new HighlightConstructor(range);
        registry.set("legado-reader-tts-current", highlight);
        highlightedWithRange = true;
      } catch {
        highlightedWithRange = false;
      }
    }

    if (highlightedWithRange || target.target) {
      const style = document.createElement("style");
      style.textContent = "::highlight(legado-reader-tts-current){background-color:rgba(255,196,0,.48);color:inherit}[data-legado-reader-tts-current=\"true\"]{background-color:rgba(255,196,0,.28)!important;outline:2px solid rgba(224,158,0,.45)!important}";
      document.head?.appendChild(style);
      ttsHighlightStyle = style;
      ttsHighlightDocument = document;
      if (!highlightedWithRange && target.target) {
        ttsHighlightElement = target.target;
        ttsHighlightPreviousValue = target.target.getAttribute("data-legado-reader-tts-current");
        target.target.setAttribute("data-legado-reader-tts-current", "true");
      }
    }

    if (range) {
      try {
        const firstCharacter = document.createRange();
        firstCharacter.setStart(target.startNode!, target.startOffset);
        firstCharacter.setEnd(target.startNode!, Math.min(target.startOffset + 1, target.startNode!.length));
        const rects = firstCharacter.getClientRects();
        const rect = rects.length ? rects[0] : firstCharacter.getBoundingClientRect();
        options.display.setPage(options.display.segmentForRect(document, rect));
        options.display.scrollToPage();
        return;
      } catch {
        // 文本范围无法测量时，仍保留段落高亮并尝试定位到段落。
      }
    }
    if (target.target) {
      options.display.setPage(options.display.segmentForElement(target.target));
      options.display.scrollToPage();
    }
  }

  async function waitForReaderTtsSegments(fromCurrentPosition = false): Promise<ReaderTtsSegment[]> {
    await nextTick();
    const frame = options.reader.frame.value;
    if (!frame) throw new Error("章节阅读画面尚未准备好，请稍后重试。");
    if (frame.contentDocument?.readyState !== "complete") {
      await new Promise<void>((resolve, reject) => {
        const finish = (error?: Error) => {
          clearTimeout(timer);
          frame.removeEventListener("load", onLoad);
          if (error) reject(error);
          else resolve();
        };
        const onLoad = () => finish();
        const timer = setTimeout(() => finish(new Error("等待章节阅读画面超时，请重试。")), 12000);
        frame.addEventListener("load", onLoad, { once: true });
        if (frame.contentDocument?.readyState === "complete") finish();
      });
    }
    const document = frame.contentDocument;
    const segments = document
      ? collectReaderTtsSegments(document, fromCurrentPosition, { width: frame.clientWidth, height: frame.clientHeight })
      : [];
    if (!segments.length) throw new Error("当前章节没有可朗读的正文。");
    return segments;
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
