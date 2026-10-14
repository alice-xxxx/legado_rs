import { computed } from "vue";
import type { ShelfResource } from "../../api/types";
import { contentLibrary, contentUrl, libraryEntries, libraryPositions } from "../books/contentLibraryState";

export const shelfBooks = computed<ShelfResource>(() => {
  const view: ShelfResource = {
    ...libraryPositions.value,
    books: libraryEntries().map(item => ({
      id: item.id, title: item.title, author: item.author,
      contentType: item.type, coverSrc: item.cover ? contentUrl(item.type, item.id, item.cover) : undefined,
      chapterCount: item.total ?? 0, latestChapter: item.latest, updatedAt: item.updatedAt ?? item.readAt,
      progress: item.readAt ? { chapterIndex: Math.max(0, (item.read ?? 1) - 1), updatedAtMs: item.readAt * 1000 } : undefined,
      bookSrc: contentUrl(item.type, item.id),
      groups: (contentLibrary.value.groups ?? []).filter(group => group.id.includes(item.id)).map(group => group.name),
    })),
  };
  const key = view.sort ?? "custom";
  if (key !== "custom") view.books.sort((a, b) => {
    const order = key === "updatedAt" ? (a.updatedAt ?? 0) - (b.updatedAt ?? 0)
      : key === "progress" ? (a.progress?.chapterIndex ?? 0) - (b.progress?.chapterIndex ?? 0)
      : String(a[key] ?? "").localeCompare(String(b[key] ?? ""));
    return view.sortOrder === "ascending" ? order : -order;
  });
  return view;
});
