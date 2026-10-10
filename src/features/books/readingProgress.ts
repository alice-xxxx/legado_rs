import type { BookResource } from "../../api/types";

/** An untouched or explicitly reset book has a zeroed progress snapshot. */
export function hasReadingProgress(progress: BookResource["progress"]): boolean {
  return Boolean(progress && (
    (progress.updatedAtMs ?? 0) > 0
    || (progress.chapterIndex ?? 0) > 0
    || (progress.offset ?? 0) > 0
    || Boolean(progress.chapterId)
  ));
}

/** Presentation only: Rust remains the sole owner of persisted progress. */
export function readingProgressPercent(progress: BookResource["progress"], chapterCount: number): number {
  if (!hasReadingProgress(progress) || !Number.isFinite(chapterCount) || chapterCount <= 0) return 0;
  const chapterIndex = progress?.chapterIndex;
  const index = typeof chapterIndex === "number" && Number.isFinite(chapterIndex)
    ? Math.max(0, chapterIndex)
    : 0;
  return Math.min(100, Math.round(((index + 1) / chapterCount) * 100));
}
