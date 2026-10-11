import type { ReadingHistoryResource, ReadingStatisticsResource } from "../../api/types";

/** Clip sessions to the requested window, retaining each session's time zone. */
export function readingStatistics(history: ReadingHistoryResource, fromMs: number, toMs: number): ReadingStatisticsResource {
  if (fromMs >= toMs || toMs - fromMs > 31 * 86_400_000) throw new Error("阅读统计时间范围无效。");
  const books = new Map<string, ReadingHistoryResource["books"][number]>();
  const days = new Map<string, ReadingHistoryResource["days"][number]>();
  let totalDurationMs = 0, totalSessions = 0;
  for (const session of history.sessions) {
    const start = Math.max(fromMs, session.endedAtMs - session.durationMs);
    const end = Math.min(toMs, session.endedAtMs);
    if (start >= end) continue;
    const duration = end - start;
    totalDurationMs += duration;
    totalSessions++;
    const book = books.get(session.bookId) ?? { bookId: session.bookId, sessionCount: 0, totalDurationMs: 0, lastReadAtMs: 0 };
    book.sessionCount++;
    book.totalDurationMs += duration;
    book.lastReadAtMs = Math.max(book.lastReadAtMs, end);
    books.set(session.bookId, book);

    const offset = session.utcOffsetMinutes * 60_000;
    for (let cursor = start; cursor < end;) {
      const dayNumber = Math.floor((cursor + offset) / 86_400_000);
      const segmentEnd = Math.min(end, (dayNumber + 1) * 86_400_000 - offset);
      const date = new Date(dayNumber * 86_400_000);
      const day = String(date.getUTCFullYear()).padStart(4, "0")
        + String(date.getUTCMonth() + 1).padStart(2, "0")
        + String(date.getUTCDate()).padStart(2, "0");
      const summary = days.get(day) ?? { day, sessionCount: 0, totalDurationMs: 0, books: [] };
      summary.sessionCount++;
      summary.totalDurationMs += segmentEnd - cursor;
      let dailyBook = summary.books.find((entry) => entry.bookId === session.bookId);
      if (!dailyBook) {
        dailyBook = { bookId: session.bookId, sessionCount: 0, totalDurationMs: 0 };
        summary.books.push(dailyBook);
      }
      dailyBook.sessionCount++;
      dailyBook.totalDurationMs += segmentEnd - cursor;
      days.set(day, summary);
      cursor = segmentEnd;
    }
  }
  const byId = (a: { bookId: string }, b: { bookId: string }) => a.bookId < b.bookId ? -1 : a.bookId > b.bookId ? 1 : 0;
  return {
    books: [...books.values()].sort((a, b) => b.lastReadAtMs - a.lastReadAtMs || byId(a, b)),
    days: [...days.values()].sort((a, b) => b.day.localeCompare(a.day)).map((day) => ({ ...day, books: day.books.sort(byId) })),
    totalDurationMs,
    totalSessions,
  };
}
