import type { ChapterPdfPageResource, ChapterTextResource, ChapterRichTextResource } from "../../api/types";
export type PdfPageResource = Pick<ChapterPdfPageResource, "src" | "pageIndex" | "defaultZoom">;
export interface ReaderTextChapter {
  index: number;
  title: string;
  content: ChapterTextResource | ChapterRichTextResource;
}

export function clamp(value: number, minimum: number, maximum: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(maximum, Math.max(minimum, number)) : minimum;
}

export function isImageOnlyChapter(raw: string): boolean {
  const document = new DOMParser().parseFromString(raw, "text/html");
  if (!document.body.querySelector("img")) return false;
  const textOnly = document.body.cloneNode(true) as HTMLElement;
  textOnly.querySelectorAll("img, picture, svg, video, audio, style, script, br, hr").forEach((node) => node.remove());
  return !textOnly.textContent?.trim();
}
