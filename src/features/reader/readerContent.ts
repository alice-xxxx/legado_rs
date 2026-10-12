import type { ChapterPdfPageResource } from "../../api/types";
export interface PdfPageResource { src: string; pageIndex: number; defaultZoom: "page-fit" | "page-width" | "actual-size" }

export function clamp(value: number, minimum: number, maximum: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(maximum, Math.max(minimum, number)) : minimum;
}

export function typedPdfPageResource(
  resource: ChapterPdfPageResource,
): PdfPageResource | null {
  if (resource.kind !== "pdfPage"
    || !Number.isInteger(resource.pageIndex)
    || resource.pageIndex < 0
    || !["page-fit", "page-width", "actual-size"].includes(resource.defaultZoom)) {
    return null;
  }

  // Rust materializes stable resource refs before this JSON reaches the
  // WebView. Treat src as the final consumable resource instead of deriving it
  // from the chapter descriptor URL or Resource Runtime path layout.
  let assetUrl: URL;
  try {
    assetUrl = new URL(resource.src);
  } catch {
    return null;
  }
  if (!["http:", "https:"].includes(assetUrl.protocol)
    || assetUrl.username || assetUrl.password) {
    return null;
  }
  return {
    src: assetUrl.href,
    pageIndex: resource.pageIndex,
    defaultZoom: resource.defaultZoom,
  };
}

export function isImageOnlyChapter(raw: string): boolean {
  const document = new DOMParser().parseFromString(raw, "text/html");
  if (!document.body.querySelector("img")) return false;
  const textOnly = document.body.cloneNode(true) as HTMLElement;
  textOnly.querySelectorAll("img, picture, svg, video, audio, style, script, br, hr").forEach((node) => node.remove());
  return !textOnly.textContent?.trim();
}
