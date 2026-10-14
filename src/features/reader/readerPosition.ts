export interface ReaderPosition { offset: number; pageId?: string; anchor?: string; quality?: string }

function ensureAnchors(viewport: HTMLElement): void {
  viewport.querySelectorAll<HTMLElement>(".chapter-markup p, .chapter-markup li, .chapter-markup h1, .chapter-markup h2, .chapter-markup pre, .chapter-markup blockquote")
    .forEach((element, index) => { if (!element.dataset.readerAnchor) element.dataset.readerAnchor = element.id || `block-${index}`; });
}
function textNodes(viewport: HTMLElement): Text[] {
  const body = viewport.querySelector(".chapter-text, .chapter-markup") ?? viewport;
  const walker = viewport.ownerDocument.createTreeWalker(body, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  while (walker.nextNode()) {
    const node = walker.currentNode as Text;
    if (node.data.length && !node.parentElement?.closest("script,style")) nodes.push(node);
  }
  return nodes;
}

function characterRect(node: Text, offset: number): DOMRect {
  const range = node.ownerDocument.createRange();
  range.setStart(node, Math.min(offset, Math.max(0, node.length - 1)));
  range.setEnd(node, Math.min(node.length, offset + 1));
  return range.getBoundingClientRect();
}

export function textPosition(viewport: HTMLElement | null, vertical: boolean): ReaderPosition {
  if (!viewport) return { offset: 0 };
  ensureAnchors(viewport);
  const bounds = viewport.getBoundingClientRect();
  let count = 0;
  for (const node of textNodes(viewport)) {
    const range = node.ownerDocument.createRange();
    range.selectNodeContents(node);
    const visible = Array.from(range.getClientRects()).some(rect => rect.width > 0 && rect.height > 0
      && rect.right > bounds.left && rect.left < bounds.right && rect.bottom > bounds.top && rect.top < bounds.bottom);
    if (visible) {
      let low = 0, high = node.length - 1;
      while (low < high) {
        const middle = Math.floor((low + high) / 2);
        const rect = characterRect(node, middle);
        const before = vertical ? rect.bottom <= bounds.top : rect.right <= bounds.left;
        if (before) low = middle + 1; else high = middle;
      }
      const block = node.parentElement?.closest<HTMLElement>("[data-reader-anchor]");
      if (block) {
        const range = node.ownerDocument.createRange();
        range.setStart(block, 0); range.setEnd(node, low);
        return { offset: range.toString().length, anchor: block.dataset.readerAnchor };
      }
      return { offset: count + low };
    }
    count += node.length;
  }
  return { offset: count };
}

export function restoreTextPosition(viewport: HTMLElement, position: ReaderPosition, vertical: boolean): void {
  let remaining = Math.max(0, position.offset);
  ensureAnchors(viewport);
  const block = Array.from(viewport.querySelectorAll<HTMLElement>("[data-reader-anchor]")).find(element => element.dataset.readerAnchor === position.anchor);
  const nodes = textNodes(viewport).filter(node => !block || block.contains(node));
  for (const node of nodes) {
    if (remaining < node.length) {
      const rect = characterRect(node, remaining);
      const bounds = viewport.getBoundingClientRect();
      const distance = vertical ? rect.top - bounds.top + viewport.scrollTop : rect.left - bounds.left + viewport.scrollLeft;
      const size = Math.max(1, vertical ? viewport.clientHeight : viewport.clientWidth);
      const page = Math.max(0, Math.floor((distance + 1) / size));
      viewport.scrollTo({ top: vertical ? page * size : 0, left: vertical ? 0 : page * size, behavior: "instant" });
      return;
    }
    remaining -= node.length;
  }
  if (!position.offset) viewport.scrollTo({ top: 0, left: 0, behavior: "instant" });
}

export function imagePosition(viewport: HTMLElement | null): ReaderPosition {
  if (!viewport) return { offset: 0 };
  const bounds = viewport.getBoundingClientRect();
  const images = Array.from(viewport.querySelectorAll<HTMLImageElement>("img[data-page-id]"));
  const image = images.find(image => image.getBoundingClientRect().bottom > bounds.top) ?? images[images.length - 1];
  if (!image) return { offset: 0 };
  const rect = image.getBoundingClientRect();
  return { pageId: image.dataset.pageId, offset: Math.round(Math.min(1, Math.max(0, (bounds.top - rect.top) / Math.max(1, rect.height))) * 1_000_000) };
}

export function restoreImagePosition(viewport: HTMLElement, position: ReaderPosition): void {
  const images = Array.from(viewport.querySelectorAll<HTMLImageElement>("img[data-page-id]"));
  const image = images.find(image => image.dataset.pageId === position.pageId) ?? images[0];
  if (!image) return;
  const rect = image.getBoundingClientRect();
  viewport.scrollTop += rect.top - viewport.getBoundingClientRect().top
    + rect.height * Math.min(1, Math.max(0, position.offset / 1_000_000));
}
