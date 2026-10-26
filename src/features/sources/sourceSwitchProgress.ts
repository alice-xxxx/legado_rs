import type { BookResource } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import { hasReadingProgress } from "../books/readingProgress";

export function sourceSwitchProgress(previous: BookResource, next: BookResource) {
  const progress = previous.progress;
  if (!hasReadingProgress(progress)) return undefined;
  const original = previous.chapters.filter(chapter => !chapter.volume);
  const chapters = next.chapters.filter(chapter => !chapter.volume);
  const current = original.find(chapter => chapter.id === progress?.chapterId)
    ?? original.find(chapter => chapter.index === progress?.chapterIndex);
  const position = current ? original.indexOf(current) : Math.max(0, progress?.chapterIndex ?? 0);
  const normalize = (title: string) => title.normalize("NFKC").toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
  const title = normalize(current?.title ?? "");
  const matches = title ? chapters.filter(chapter => normalize(chapter.title) === title) : [];
  const chapter = matches.length === 1 ? matches[0] : chapters[Math.min(position, chapters.length - 1)];
  if (!chapter) throw new Error("新来源没有可阅读的章节，无法继承阅读进度。");
  return {
    chapterId: bookContentType(next) === "pdf" ? null : chapter.id,
    chapterIndex: chapter.index,
    offset: progress?.offset ?? 0,
    // 正文锚点可用于近似恢复段落；图片页 ID 和清晰度属于旧来源，不复制。
    anchor: progress?.anchor,
    updatedAtMs: Math.max(Date.now(), (next.progress?.updatedAtMs ?? 0) + 1),
  };
}
