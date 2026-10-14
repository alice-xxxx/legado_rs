import type { ContentType } from "./types";
export type BookContentType = ContentType;

/** Rust's contentType owns reader selection; kind is normally a genre label. */
export function bookContentType(book: {
  contentType?: string; mediaType?: string; kind?: string; type?: unknown;
} | null | undefined): BookContentType | undefined {
  if (!book) return undefined;
  for (const value of [book.contentType, book.mediaType, book.kind, book.type]) {
    if (typeof value !== "string") continue;
    switch (value.trim().toLowerCase()) {
      case "text": case "novel": case "小说": return "text";
      case "comic": case "image": case "manga": case "manhua": case "manhwa": case "漫画": return "image";
      case "html": case "richtext": return "html";
      case "pdf": return "pdf";
      case "audio": case "音频": return "audio";
      case "video": case "视频": return "video";
    }
  }
  return undefined;
}
