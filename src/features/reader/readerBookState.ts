import { ref } from "vue";
import { readBook, readChapter } from "../../api/database";
import type { BookResource, ChapterResource } from "../../api/types";

// 阅读和播放独立持有作品，浏览其他详情页不会替换当前会话。
export const readingBook = ref<BookResource | null>(null);
const chapterLoads = new Map<string, { book: BookResource; dirty: boolean; task: Promise<ChapterResource | null> }>();

export async function refreshReadingChapter(bookId: string, chapterId: string): Promise<ChapterResource | null> {
  const book = readingBook.value;
  if (book?.id !== bookId) return null;
  const key = `${bookId}:${chapterId}`;
  const pending = chapterLoads.get(key);
  if (pending?.book === book) { pending.dirty = true; return pending.task; }
  const load = { book, dirty: false, task: null as unknown as Promise<ChapterResource | null> };
  load.task = (async () => {
    let chapter: ChapterResource | null = null;
    do {
      load.dirty = false;
      const snapshot = await readChapter(book, chapterId);
      if (!snapshot || readingBook.value !== book) return null;
      if (snapshot.contentType !== book.contentType) {
        const updated = await readBook(bookId);
        if (readingBook.value !== book) return null;
        readingBook.value = updated;
        return updated.chapters.find(entry => entry.id === chapterId) ?? null;
      }
      chapter = snapshot.chapter;
      const index = book.chapters.findIndex(entry => entry.id === chapterId && entry.index === chapter!.index);
      if (index < 0) return null;
      book.chapters[index] = chapter;
    } while (load.dirty);
    return chapter;
  })().finally(() => { if (chapterLoads.get(key) === load) chapterLoads.delete(key); });
  chapterLoads.set(key, load);
  return load.task;
}
