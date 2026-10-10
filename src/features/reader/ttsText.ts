export interface ReaderTtsPiece {
  node: Text;
  nodeStart: number;
  flatStart: number;
  flatEnd: number;
}

export interface ReaderTtsSegment {
  text: string;
  target: HTMLElement | null;
  pieces: ReaderTtsPiece[];
  textOffsets: number[];
}

export interface ReaderTtsChunkTarget {
  target: HTMLElement | null;
  startNode: Text | null;
  startOffset: number;
  endNode: Text | null;
  endOffset: number;
}

export interface ReaderTtsViewportFallback {
  width: number;
  height: number;
}

/** 从阅读文档中收集可见正文，并保留朗读文本到原 DOM 的偏移映射。 */
export function collectReaderTtsSegments(
  document: Document,
  fromCurrentPosition: boolean,
  viewportFallback: ReaderTtsViewportFallback,
): ReaderTtsSegment[] {
  const body = document.body;
  const viewportWidth = Math.max(1, document.documentElement.clientWidth || viewportFallback.width || 1);
  const viewportHeight = Math.max(1, document.documentElement.clientHeight || viewportFallback.height || 1);
  const isInViewport = (rect: DOMRect): boolean => rect.left < viewportWidth
    && rect.right > 0
    && rect.top < viewportHeight
    && rect.bottom > 0;
  // Rust 的阅读 HTML 会用 <br> 分隔相邻正文段；朗读时也以它作为段落边界。
  const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
    acceptNode(node) {
      if (node.nodeType === 1) {
        const element = node as HTMLElement;
        if (element.tagName !== "BR") return NodeFilter.FILTER_SKIP;
        const parent = element.parentElement;
        if (!parent || parent.closest("script,style,noscript,textarea,select,button,audio,video,svg,[hidden]")) {
          return NodeFilter.FILTER_REJECT;
        }
        const style = document.defaultView?.getComputedStyle(parent);
        return style?.display === "none" || style?.visibility === "hidden"
          ? NodeFilter.FILTER_REJECT
          : NodeFilter.FILTER_ACCEPT;
      }
      const text = node as Text;
      const parent = text.parentElement;
      if (!parent || !text.data.trim() || parent.closest("script,style,noscript,textarea,select,button,audio,video,svg,[hidden]")) {
        return NodeFilter.FILTER_REJECT;
      }
      const style = document.defaultView?.getComputedStyle(parent);
      if (style?.display === "none" || style?.visibility === "hidden") return NodeFilter.FILTER_REJECT;
      const range = document.createRange();
      range.selectNodeContents(text);
      return range.getClientRects().length ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });

  let startNode: Text | null = null;
  let startOffset = 0;
  if (fromCurrentPosition) {
    while (walker.nextNode()) {
      if (walker.currentNode.nodeType !== 3) continue;
      const text = walker.currentNode as Text;
      const range = document.createRange();
      range.selectNodeContents(text);
      if (![...range.getClientRects()].some(isInViewport)) continue;

      let low = 0;
      let high = text.length;
      while (low < high) {
        const middle = Math.floor((low + high) / 2);
        range.setStart(text, 0);
        range.setEnd(text, middle + 1);
        if ([...range.getClientRects()].some(isInViewport)) high = middle;
        else low = middle + 1;
      }
      startNode = text;
      startOffset = low;
      break;
    }
  } else {
    while (walker.nextNode()) {
      if (walker.currentNode.nodeType !== 3) continue;
      startNode = walker.currentNode as Text;
      break;
    }
  }
  if (!startNode) throw new Error(fromCurrentPosition
    ? "当前位置没有可朗读的正文，请翻到有文字的页面后重试。"
    : "当前章节没有可朗读的正文。");

  const segments: ReaderTtsSegment[] = [];
  let currentTarget: HTMLElement | null = null;
  let currentText = "";
  let currentPieces: ReaderTtsPiece[] = [];
  const flush = () => {
    const normalized = normalizeTtsText(currentText);
    if (normalized.text) segments.push({
      text: normalized.text,
      target: currentTarget ?? body,
      pieces: currentPieces,
      textOffsets: normalized.offsets,
    });
    currentText = "";
    currentPieces = [];
  };
  let currentNode: Node | null = startNode;
  while (currentNode) {
    if (currentNode.nodeType === 1) {
      flush();
      currentNode = walker.nextNode();
      continue;
    }
    const text = currentNode as Text;
    const parent = text.parentElement;
    const block = parent?.closest<HTMLElement>("p,li,blockquote,pre,h1,h2,h3,h4,h5,h6,div,section,article,td,th") ?? body;
    if (currentTarget && block !== currentTarget) flush();
    currentTarget = block;
    const nodeStart = currentNode === startNode ? startOffset : 0;
    const content = text.data.slice(nodeStart);
    if (content) {
      const flatStart = currentText.length;
      currentText += content;
      currentPieces.push({ node: text, nodeStart, flatStart, flatEnd: currentText.length });
    }
    currentNode = walker.nextNode();
  }
  flush();
  return segments;
}

/** 折叠空白字符，同时记录规范化字符对应的原文位置。 */
function normalizeTtsText(raw: string): { text: string; offsets: number[] } {
  const characters: string[] = [];
  const offsets: number[] = [];
  for (let index = 0; index < raw.length; index += 1) {
    const character = raw[index];
    if (/\s/.test(character)) {
      if (characters.length && characters[characters.length - 1] !== " ") {
        characters.push(" ");
        offsets.push(index);
      }
      continue;
    }
    characters.push(character);
    offsets.push(index);
  }
  while (characters[characters.length - 1] === " ") {
    characters.pop();
    offsets.pop();
  }
  return { text: characters.join(""), offsets };
}

/** 按服务可接受的长度切分文本，并优先在句末标点处断开。 */
export function splitTtsText(text: string): Array<{ text: string; start: number; end: number }> {
  const chunks: Array<{ text: string; start: number; end: number }> = [];
  let cursor = 0;
  const maximumLength = 1600;
  while (text.length - cursor > maximumLength) {
    let cut = -1;
    const minimumCut = Math.floor(maximumLength * 0.65);
    for (const mark of ["\n", "。", "！", "？", "!", "?", "；", ";"]) {
      const candidate = text.lastIndexOf(mark, cursor + maximumLength - 1);
      if (candidate >= cursor + minimumCut && candidate > cut) cut = candidate + 1;
    }
    if (cut < cursor + minimumCut) cut = cursor + maximumLength;
    if (cut < text.length && /[\uD800-\uDBFF]/.test(text[cut - 1]) && /[\uDC00-\uDFFF]/.test(text[cut])) cut -= 1;
    let start = cursor;
    let end = cut;
    while (start < end && /\s/.test(text[start])) start += 1;
    while (end > start && /\s/.test(text[end - 1])) end -= 1;
    if (end > start) chunks.push({ text: text.slice(start, end), start, end });
    cursor = cut;
    while (cursor < text.length && /\s/.test(text[cursor])) cursor += 1;
  }
  let start = cursor;
  let end = text.length;
  while (start < end && /\s/.test(text[start])) start += 1;
  while (end > start && /\s/.test(text[end - 1])) end -= 1;
  if (end > start) chunks.push({ text: text.slice(start, end), start, end });
  return chunks;
}

/** 将文本片段映射回 DOM 范围，供朗读进度高亮使用。 */
export function mapTtsSegmentChunks(segment: ReaderTtsSegment): Array<{ text: string; target: ReaderTtsChunkTarget }> {
  const chunks: Array<{ text: string; target: ReaderTtsChunkTarget }> = [];
  for (const chunk of splitTtsText(segment.text)) {
    const rawStart = segment.textOffsets[chunk.start];
    const rawEnd = segment.textOffsets[chunk.end - 1];
    const startPiece = segment.pieces.find((piece) => rawStart >= piece.flatStart && rawStart < piece.flatEnd);
    const endPiece = [...segment.pieces].reverse().find((piece) => rawEnd >= piece.flatStart && rawEnd < piece.flatEnd);
    chunks.push({
      text: chunk.text,
      target: startPiece && endPiece ? {
        target: segment.target,
        startNode: startPiece.node,
        startOffset: startPiece.nodeStart + rawStart - startPiece.flatStart,
        endNode: endPiece.node,
        endOffset: endPiece.nodeStart + rawEnd - endPiece.flatStart + 1,
      } : { target: segment.target, startNode: null, startOffset: 0, endNode: null, endOffset: 0 },
    });
  }
  return chunks;
}
