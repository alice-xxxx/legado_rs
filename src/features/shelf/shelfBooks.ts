import { computed, watch } from "vue";
import type { ShelfResource } from "../../api/types";
import { bookPreviews, loadBookPreviews } from "../books/bookDetailState";
import { shelf } from "./shelfStore";

// Read-only display: changes to membership, order and groups go directly to shelf.
watch(() => shelf.value.books.map(book => book.id), ids => { void loadBookPreviews(ids); }, { immediate: true });
export const shelfBooks = computed<ShelfResource>(() => {
  const view: ShelfResource = {
    ...shelf.value,
    books: shelf.value.books.flatMap(position => {
      const book = bookPreviews.value[position.id];
      return book ? [{ ...book, groups: position.groups }] : [];
    }),
  };
  const key = view.sort ?? "updatedAt";
  if (key !== "custom") view.books.sort((a, b) => {
    const order = key === "updatedAt" ? (a.progress?.updatedAtMs ?? 0) - (b.progress?.updatedAtMs ?? 0)
      : key === "progress" ? (a.progress?.chapterIndex ?? 0) - (b.progress?.chapterIndex ?? 0)
      : String(a[key] ?? "").localeCompare(String(b[key] ?? ""));
    return view.sortOrder === "ascending" ? order : -order;
  });
  return view;
});
