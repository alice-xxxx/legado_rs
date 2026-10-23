import type { BookResource } from "../../api/types";

export const bookInformationMaxAgeMs = 30 * 60 * 1000;

export function hasBookInformation(book: BookResource): boolean {
  return !book.sourceId || book.informationFetchedAtMs != null;
}

/** 所有详情入口共用有效期；传入的最新章节只是额外的更新信号。 */
export function shouldRefreshBookInformation(book: BookResource, latestChapter?: string, now = Date.now()): boolean {
  if (!book.sourceId || !book.bookUrl) return false;
  const fetchedAt = book.informationFetchedAtMs;
  if (!fetchedAt || fetchedAt > now || now - fetchedAt >= bookInformationMaxAgeMs) return true;
  const normalize = (title: string | undefined) => title?.replace(/\s+/g, " ").trim() ?? "";
  const latest = normalize(latestChapter);
  return Boolean(latest && latest !== normalize(book.latestChapter));
}
