import { shallowRef } from "vue";
import type { BookResource } from "../../api/types";

/** The book shown on the detail screen is also the reader host's start source. */
export const openedBook = shallowRef<BookResource | null>(null);
